//! Shared Postgres persistence and leases for workflow instances.

use std::collections::HashMap;
use std::sync::Arc;

use pylon_storage::pg_datastore::PgPool;

use crate::workflows::{
    BufferedEvent, EventDelivery, StatusFilter, StepResult, StepStatus, WorkflowCounts,
    WorkflowFilter, WorkflowInstance, WorkflowStatus,
};

const WORKFLOW_COLUMNS: &str = "id,name,input,status,output,error,created_at,started_at,\
     completed_at,wake_at,waiting_for,current_step,max_retries,key,wait_deadline,cancel_reason";
const TERMINAL: &str = "('Completed','Failed','Cancelled')";

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
                );
                ALTER TABLE _pylon_workflows
                    ADD COLUMN IF NOT EXISTS key TEXT,
                    ADD COLUMN IF NOT EXISTS wait_deadline BIGINT,
                    ADD COLUMN IF NOT EXISTS cancel_reason TEXT;
                CREATE UNIQUE INDEX IF NOT EXISTS _pylon_workflows_active_key_idx
                    ON _pylon_workflows (name, key)
                    WHERE key IS NOT NULL
                      AND status NOT IN ('Completed','Failed','Cancelled');
                CREATE INDEX IF NOT EXISTS _pylon_workflows_key_idx
                    ON _pylon_workflows (key) WHERE key IS NOT NULL;
                CREATE INDEX IF NOT EXISTS _pylon_workflows_wait_idx
                    ON _pylon_workflows (wait_deadline)
                    WHERE status = 'WaitingForEvent';
                CREATE TABLE IF NOT EXISTS _pylon_workflow_events (
                    seq BIGSERIAL PRIMARY KEY,
                    workflow_id TEXT NOT NULL REFERENCES _pylon_workflows(id) ON DELETE CASCADE,
                    event TEXT NOT NULL,
                    data JSONB NOT NULL,
                    received_at BIGINT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS _pylon_workflow_events_wf_idx
                    ON _pylon_workflow_events (workflow_id, event, seq);",
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
        let wait_deadline = workflow
            .wait_deadline
            .map(|v| v.min(i64::MAX as u64) as i64);
        let consumed: Vec<i64> = workflow
            .consumed_events
            .iter()
            .map(|&seq| seq.min(i64::MAX as u64) as i64)
            .collect();
        let saved = self.pool.with_client(|client| {
            let mut tx = client.transaction()?;
            let changed = if let Some(token) = token {
                tx.execute(
                    "UPDATE _pylon_workflows SET
                         name=$2,input=$3,status=$4,output=$5,error=$6,created_at=$7,
                         started_at=$8,completed_at=$9,wake_at=$10,waiting_for=$11,
                         current_step=$12,max_retries=$13,key=$15,wait_deadline=$16,
                         cancel_reason=$17,version=version+1
                     WHERE id=$1 AND lease_token=$14 AND status <> 'Cancelled'",
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
                        &workflow.key,
                        &wait_deadline,
                        &workflow.cancel_reason,
                    ],
                )?
            } else {
                tx.execute(
                    "INSERT INTO _pylon_workflows
                     (id,name,input,status,output,error,created_at,started_at,completed_at,
                      wake_at,waiting_for,current_step,max_retries,key,wait_deadline,
                      cancel_reason)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
                     ON CONFLICT (id) DO UPDATE SET
                         name=EXCLUDED.name,input=EXCLUDED.input,status=EXCLUDED.status,
                         output=EXCLUDED.output,error=EXCLUDED.error,
                         started_at=EXCLUDED.started_at,completed_at=EXCLUDED.completed_at,
                         wake_at=EXCLUDED.wake_at,waiting_for=EXCLUDED.waiting_for,
                         current_step=EXCLUDED.current_step,max_retries=EXCLUDED.max_retries,
                         key=EXCLUDED.key,wait_deadline=EXCLUDED.wait_deadline,
                         cancel_reason=EXCLUDED.cancel_reason,
                         version=_pylon_workflows.version+1
                     WHERE _pylon_workflows.status <> 'Cancelled'",
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
                        &workflow.key,
                        &wait_deadline,
                        &workflow.cancel_reason,
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
            insert_steps(&mut tx, &workflow.id, &workflow.steps)?;
            // Buffered events consumed by this transition. Other processes
            // insert into the inbox without the lease, so only the consumed
            // rows are deleted; the inbox is never rewritten from memory.
            if !consumed.is_empty() {
                tx.execute(
                    "DELETE FROM _pylon_workflow_events WHERE workflow_id=$1 AND seq = ANY($2)",
                    &[&workflow.id, &consumed],
                )?;
            }
            tx.commit()?;
            Ok(true)
        })?;
        if !saved {
            return Err("workflow lease is no longer owned or the run was cancelled".into());
        }
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Option<WorkflowInstance>, String> {
        self.pool.with_client(|client| {
            let row = client.query_opt(
                &format!("SELECT {WORKFLOW_COLUMNS} FROM _pylon_workflows WHERE id=$1"),
                &[&id],
            )?;
            match row {
                Some(row) => {
                    let mut workflow = row_to_workflow(&row);
                    workflow.steps = load_steps(client, id)?;
                    workflow.pending_events = load_events(client, id)?;
                    Ok(Some(workflow))
                }
                None => Ok(None),
            }
        })
    }

    pub fn status_counts(&self) -> Result<WorkflowCounts, String> {
        self.pool.with_client(load_status_counts)
    }

    pub fn list(&self, status: Option<&str>) -> Result<Vec<WorkflowInstance>, String> {
        self.pool.with_client(|client| {
            let rows = client.query(
                &format!(
                    "SELECT {WORKFLOW_COLUMNS} FROM _pylon_workflows
                     WHERE ($1::TEXT IS NULL OR status=$1)
                     ORDER BY created_at DESC"
                ),
                &[&status],
            )?;
            let mut workflows: Vec<_> = rows.iter().map(row_to_workflow).collect();
            load_history(client, &mut workflows, true, false)?;
            Ok(workflows)
        })
    }

    /// Insert a new run. When the run has a key and a non-terminal run
    /// with the same name and key exists, nothing is inserted and that
    /// run's id is returned.
    pub fn insert_new(&self, workflow: &WorkflowInstance) -> Result<Option<String>, String> {
        let created_at = parse_stamp_i64(&workflow.created_at);
        let max_retries = workflow.max_retries.min(i32::MAX as u32) as i32;
        let status = workflow_status_to_str(&workflow.status);
        // Two tries: the conflicting run can finish between the insert
        // and the lookup, which frees the key.
        for _ in 0..2 {
            let outcome = self.pool.with_client(|client| {
                let inserted = client.execute(
                    "INSERT INTO _pylon_workflows
                     (id,name,input,status,created_at,current_step,max_retries,key)
                     VALUES ($1,$2,$3,$4,$5,0,$6,$7)
                     ON CONFLICT DO NOTHING",
                    &[
                        &workflow.id,
                        &workflow.name,
                        &workflow.input,
                        &status,
                        &created_at,
                        &max_retries,
                        &workflow.key,
                    ],
                )?;
                if inserted == 1 {
                    return Ok(Some(None));
                }
                let existing = client.query_opt(
                    &format!(
                        "SELECT id FROM _pylon_workflows
                         WHERE name=$1 AND key=$2 AND status NOT IN {TERMINAL}"
                    ),
                    &[&workflow.name, &workflow.key],
                )?;
                Ok(existing.map(|row| Some(row.get::<_, String>(0))))
            })?;
            if let Some(result) = outcome {
                return Ok(result);
            }
        }
        Err(format!(
            "could not start workflow '{}': key conflict did not resolve",
            workflow.name
        ))
    }

    /// Add an event to a run's inbox without taking the lease. Returns
    /// whether the run is waiting for it right now.
    pub fn push_event(
        &self,
        id: &str,
        event: &str,
        data: &serde_json::Value,
        max_buffered: usize,
    ) -> Result<EventDelivery, String> {
        let now = now_secs_i64();
        let max_buffered = max_buffered.min(i64::MAX as usize) as i64;
        let outcome = self.pool.with_client(|client| {
            let mut tx = client.transaction()?;
            // FOR UPDATE serializes senders to one run (so two cannot
            // pass the capacity check for the same last slot) and orders
            // this insert against a concurrent cancel: an event never
            // lands after the cancel commits.
            let Some(row) = tx.query_opt(
                "SELECT status, waiting_for, wait_deadline FROM _pylon_workflows
                 WHERE id=$1 FOR UPDATE",
                &[&id],
            )?
            else {
                return Ok(Err(format!("Workflow '{id}' not found")));
            };
            let status = workflow_status_from_str(row.get::<_, String>(0).as_str());
            let waiting_for: Option<String> = row.get(1);
            let wait_deadline: Option<i64> = row.get(2);
            if status.is_terminal() {
                return Ok(Err(format!(
                    "Workflow is {}; it no longer accepts events",
                    status.api_name()
                )));
            }
            let buffered: i64 = tx
                .query_one(
                    "SELECT COUNT(*) FROM _pylon_workflow_events WHERE workflow_id=$1",
                    &[&id],
                )?
                .get(0);
            if buffered >= max_buffered {
                return Ok(Err(format!(
                    "Workflow has {max_buffered} buffered events; it must consume some before more can be sent"
                )));
            }
            tx.execute(
                "INSERT INTO _pylon_workflow_events (workflow_id,event,data,received_at)
                 VALUES ($1,$2,$3,$4)",
                &[&id, &event, data, &now],
            )?;
            tx.commit()?;
            let delivered = status == WorkflowStatus::WaitingForEvent
                && waiting_for.as_deref() == Some(event)
                && wait_deadline.is_none_or(|d| now <= d);
            Ok(Ok(EventDelivery {
                delivered,
                buffered: !delivered,
            }))
        })?;
        outcome
    }

    /// Insert stored events for a run as they were, with their receive
    /// times. Used to copy a SQLite store; [`Self::push_event`] is the path
    /// for new events. Postgres numbers the events again, in the given order.
    pub fn restore_events(&self, id: &str, events: &[BufferedEvent]) -> Result<(), String> {
        if events.is_empty() {
            return Ok(());
        }
        self.pool.with_client(|client| {
            let mut tx = client.transaction()?;
            for event in events {
                let received_at = parse_stamp_i64(&event.received_at);
                tx.execute(
                    "INSERT INTO _pylon_workflow_events (workflow_id,event,data,received_at)
                     VALUES ($1,$2,$3,$4)",
                    &[&id, &event.event, &event.data, &received_at],
                )?;
            }
            tx.commit()
        })
    }

    pub fn load_events(&self, id: &str) -> Result<Vec<BufferedEvent>, String> {
        self.pool.with_client(|client| load_events(client, id))
    }

    /// Cancel a run without taking the lease. Returns false when the run
    /// had already finished. The Cancelled status fences later writes
    /// from a driver that still holds the lease (see `save_inner`).
    pub fn cancel(&self, id: &str, reason: Option<&str>, now: u64) -> Result<bool, String> {
        let now = now.min(i64::MAX as u64) as i64;
        let outcome = self.pool.with_client(|client| {
            let changed = client.execute(
                &format!(
                    "UPDATE _pylon_workflows
                     SET status='Cancelled',completed_at=$2,cancel_reason=$3,
                         waiting_for=NULL,wait_deadline=NULL,wake_at=NULL,
                         version=version+1
                     WHERE id=$1 AND status NOT IN {TERMINAL}"
                ),
                &[&id, &now, &reason],
            )?;
            if changed == 1 {
                return Ok(Ok(true));
            }
            let exists = client
                .query_opt("SELECT 1 FROM _pylon_workflows WHERE id=$1", &[&id])?
                .is_some();
            Ok(if exists {
                Ok(false)
            } else {
                Err(format!("Workflow '{id}' not found"))
            })
        })?;
        outcome
    }

    pub fn list_filtered(
        &self,
        filter: &WorkflowFilter,
        limit: usize,
        include_steps: bool,
    ) -> Result<Vec<WorkflowInstance>, String> {
        let (status, active) = match &filter.status {
            Some(StatusFilter::Is(s)) => (Some(workflow_status_to_str(s)), false),
            Some(StatusFilter::Active) => (None, true),
            None => (None, false),
        };
        let limit = limit.min(i64::MAX as usize) as i64;
        self.pool.with_client(|client| {
            let rows = client.query(
                &format!(
                    "SELECT {WORKFLOW_COLUMNS} FROM _pylon_workflows
                     WHERE ($1::TEXT IS NULL OR status=$1)
                       AND (NOT $2 OR status NOT IN {TERMINAL})
                       AND ($3::TEXT IS NULL OR name=$3)
                       AND ($4::TEXT IS NULL OR key=$4)
                     ORDER BY created_at DESC, id DESC
                     LIMIT $5"
                ),
                &[&status, &active, &filter.name, &filter.key, &limit],
            )?;
            let mut workflows: Vec<_> = rows.iter().map(row_to_workflow).collect();
            load_history(client, &mut workflows, include_steps, true)?;
            Ok(workflows)
        })
    }

    /// Whether the run has work right now: runnable, a sleep that is due,
    /// or a wait that timed out or has a matching buffered event.
    pub fn has_due_work(&self, id: &str, now: u64) -> Result<bool, String> {
        let now = now.min(i64::MAX as u64) as i64;
        self.pool.with_client(|client| {
            client
                .query_opt(
                    "SELECT 1 FROM _pylon_workflows w WHERE w.id=$1 AND (
                         w.status IN ('Pending','Running')
                         OR (w.status='Sleeping' AND (w.wake_at IS NULL OR w.wake_at <= $2))
                         OR (w.status='WaitingForEvent' AND (
                             (w.wait_deadline IS NOT NULL AND w.wait_deadline <= $2)
                             OR EXISTS (SELECT 1 FROM _pylon_workflow_events e
                                        WHERE e.workflow_id=w.id AND e.event=w.waiting_for
                                   AND (w.wait_deadline IS NULL OR e.received_at <= w.wait_deadline)))))",
                    &[&id, &now],
                )
                .map(|row| row.is_some())
        })
    }

    /// Waiting runs whose timeout passed or that have a matching
    /// buffered event.
    pub fn due_wait_ids(&self, now: u64) -> Result<Vec<String>, String> {
        let now = now.min(i64::MAX as u64) as i64;
        self.pool.with_client(|client| {
            client
                .query(
                    "SELECT w.id FROM _pylon_workflows w
                     WHERE w.status='WaitingForEvent' AND (
                         (w.wait_deadline IS NOT NULL AND w.wait_deadline <= $1)
                         OR EXISTS (SELECT 1 FROM _pylon_workflow_events e
                                    WHERE e.workflow_id=w.id AND e.event=w.waiting_for
                                   AND (w.wait_deadline IS NULL OR e.received_at <= w.wait_deadline)))",
                    &[&now],
                )
                .map(|rows| rows.into_iter().map(|row| row.get(0)).collect())
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

const STEP_BATCH_ROWS: usize = 256;
const STEP_BATCH_BYTES: usize = 1024 * 1024;

fn load_status_counts<C: pylon_storage::pg_exec::PgConn>(
    client: &mut C,
) -> Result<WorkflowCounts, postgres::Error> {
    let mut counts = WorkflowCounts::default();
    for row in client.query(
        "SELECT status, COUNT(*) FROM _pylon_workflows GROUP BY status",
        &[],
    )? {
        counts.add(
            &workflow_status_from_str(row.get(0)),
            row.get::<_, i64>(1) as u64,
        );
    }
    Ok(counts)
}

fn insert_steps<C: pylon_storage::pg_exec::PgConn>(
    tx: &mut C,
    workflow_id: &str,
    steps: &[StepResult],
) -> Result<(), postgres::Error> {
    let mut start = 0;
    let mut bytes = 0usize;
    for (index, step) in steps.iter().enumerate() {
        struct CountBytes(usize);
        impl std::io::Write for CountBytes {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self.0.saturating_add(bytes.len());
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut size = CountBytes(
            workflow_id
                .len()
                .saturating_add(step.step_id.len())
                .saturating_add(step.name.len())
                .saturating_add(step.error.as_ref().map_or(0, String::len))
                .saturating_add(128),
        );
        if let Some(output) = &step.output {
            serde_json::to_writer(&mut size, output).expect("counting JSON bytes cannot fail");
        }
        if index > start
            && (index - start == STEP_BATCH_ROWS || bytes.saturating_add(size.0) > STEP_BATCH_BYTES)
        {
            insert_step_batch(tx, workflow_id, start, &steps[start..index])?;
            start = index;
            bytes = 0;
        }
        bytes = bytes.saturating_add(size.0);
    }
    if start < steps.len() {
        insert_step_batch(tx, workflow_id, start, &steps[start..])?;
    }
    Ok(())
}

fn insert_step_batch<C: pylon_storage::pg_exec::PgConn>(
    tx: &mut C,
    workflow_id: &str,
    start: usize,
    steps: &[StepResult],
) -> Result<(), postgres::Error> {
    use std::fmt::Write;
    let converted: Vec<_> = steps
        .iter()
        .enumerate()
        .map(|(i, step)| {
            (
                start.saturating_add(i).min(i64::MAX as usize) as i64,
                step_status_to_str(&step.status),
                step.started_at.as_deref().map(parse_stamp_i64),
                step.completed_at.as_deref().map(parse_stamp_i64),
                step.duration_ms.map(|v| v.min(i64::MAX as u64) as i64),
                step.retry_count.min(i32::MAX as u32) as i32,
            )
        })
        .collect();
    let mut sql = String::from(
        "INSERT INTO _pylon_workflow_steps
         (workflow_id,step_index,step_id,name,status,output,error,started_at,
          completed_at,duration_ms,retry_count) VALUES ",
    );
    let mut params: Vec<&(dyn postgres::types::ToSql + Sync)> =
        Vec::with_capacity(steps.len() * 11);
    for (i, (step, (index, status, started_at, completed_at, duration_ms, retry_count))) in
        steps.iter().zip(&converted).enumerate()
    {
        if i > 0 {
            sql.push(',');
        }
        sql.push('(');
        for column in 0..11 {
            if column > 0 {
                sql.push(',');
            }
            write!(sql, "${}", i * 11 + column + 1).unwrap();
        }
        sql.push(')');
        params.extend_from_slice(&[
            &workflow_id,
            index,
            &step.step_id,
            &step.name,
            status,
            &step.output,
            &step.error,
            started_at,
            completed_at,
            duration_ms,
            retry_count,
        ]);
    }
    tx.execute(&sql, &params)?;
    Ok(())
}

fn load_history<C: pylon_storage::pg_exec::PgConn>(
    client: &mut C,
    workflows: &mut [WorkflowInstance],
    include_steps: bool,
    include_events: bool,
) -> Result<(), postgres::Error> {
    // Bound recovery reads as well as the paginated list path.
    for workflows in workflows.chunks_mut(1000) {
        let ids: Vec<_> = workflows
            .iter()
            .map(|workflow| workflow.id.clone())
            .collect();
        let indexes: HashMap<_, _> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        if include_steps {
            for workflow in workflows.iter_mut() {
                workflow.steps.clear();
            }
            client.query_each(
                "SELECT step_id,name,status,output,error,started_at,completed_at,
                    duration_ms,retry_count,workflow_id
             FROM _pylon_workflow_steps WHERE workflow_id = ANY($1::text[])
             ORDER BY workflow_id,step_index",
                &[&ids],
                &mut |row| {
                    let id: &str = row.get(9);
                    if let Some(&index) = indexes.get(id) {
                        workflows[index].steps.push(row_to_step(&row));
                    }
                },
            )?;
        }
        if include_events {
            for workflow in workflows.iter_mut() {
                workflow.pending_events.clear();
            }
            client.query_each(
                "SELECT seq,event,data,received_at,workflow_id FROM _pylon_workflow_events
             WHERE workflow_id = ANY($1::text[]) ORDER BY workflow_id,seq",
                &[&ids],
                &mut |row| {
                    let id: &str = row.get(4);
                    if let Some(&index) = indexes.get(id) {
                        workflows[index].pending_events.push(row_to_event(&row));
                    }
                },
            )?;
        }
    }
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

fn load_events(
    client: &mut postgres::Client,
    workflow_id: &str,
) -> Result<Vec<BufferedEvent>, postgres::Error> {
    client
        .query(
            "SELECT seq,event,data,received_at FROM _pylon_workflow_events
             WHERE workflow_id=$1 ORDER BY seq",
            &[&workflow_id],
        )
        .map(|rows| rows.iter().map(row_to_event).collect())
}

fn row_to_event(row: &postgres::Row) -> BufferedEvent {
    BufferedEvent {
        seq: row.get::<_, i64>(0).max(0) as u64,
        event: row.get(1),
        data: row.get(2),
        received_at: stamp(row.get(3)),
    }
}

fn now_secs_i64() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
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
        key: row.get(13),
        wait_deadline: row.get::<_, Option<i64>>(14).map(|v| v.max(0) as u64),
        pending_events: Vec::new(),
        cancel_reason: row.get(15),
        consumed_events: Vec::new(),
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
            key: None,
            wait_deadline: None,
            pending_events: Vec::new(),
            cancel_reason: None,
            consumed_events: Vec::new(),
        }
    }

    struct CountingReads<'a> {
        client: &'a mut postgres::Client,
        queries: usize,
    }

    impl pylon_storage::pg_exec::PgConn for CountingReads<'_> {
        fn execute(
            &mut self,
            _: &str,
            _: &[&(dyn postgres::types::ToSql + Sync)],
        ) -> Result<u64, postgres::Error> {
            panic!("history reads must not write");
        }
        fn query(
            &mut self,
            _: &str,
            _: &[&(dyn postgres::types::ToSql + Sync)],
        ) -> Result<Vec<postgres::Row>, postgres::Error> {
            panic!("history reads must stream rows");
        }
        fn query_opt(
            &mut self,
            _: &str,
            _: &[&(dyn postgres::types::ToSql + Sync)],
        ) -> Result<Option<postgres::Row>, postgres::Error> {
            panic!("history reads must use a batch");
        }
        fn query_each(
            &mut self,
            sql: &str,
            params: &[&(dyn postgres::types::ToSql + Sync)],
            visit: &mut dyn FnMut(postgres::Row),
        ) -> Result<(), postgres::Error> {
            self.queries += 1;
            pylon_storage::pg_exec::PgConn::query_each(self.client, sql, params, visit)
        }
    }

    #[test]
    fn list_batches_history_and_preserves_child_order() {
        let Some(pool) = test_pool() else {
            return;
        };
        let suffix = pylon_cluster::new_instance_id();
        let store = PgWorkflowStore::open(Arc::clone(&pool), format!("list_{suffix}")).unwrap();
        let name = format!("list_{suffix}");
        let mut ids = Vec::new();
        for i in 0..12 {
            let mut workflow = test_workflow(format!("wf_{suffix}_{i:02}"));
            workflow.name = name.clone();
            workflow.created_at = format!("{}Z", i / 2);
            let step = workflow.steps[0].clone();
            workflow.steps = (0..i % 3)
                .map(|n| StepResult {
                    step_id: format!("step_{n}"),
                    output: Some(serde_json::json!({"workflow":i,"step":n})),
                    ..step.clone()
                })
                .collect();
            store.save(&workflow).unwrap();
            ids.push(workflow.id);
        }
        for round in 0..3 {
            for (i, id) in ids.iter().enumerate() {
                if i % 4 > round {
                    store
                        .restore_events(
                            id,
                            &[BufferedEvent {
                                seq: 0,
                                event: format!("event_{round}"),
                                data: serde_json::json!({"workflow":i,"round":round}),
                                received_at: format!("{round}Z"),
                            }],
                        )
                        .unwrap();
                }
            }
        }
        for include_steps in [false, true] {
            let rows = store
                .list_filtered(
                    &WorkflowFilter {
                        name: Some(name.clone()),
                        ..Default::default()
                    },
                    8,
                    include_steps,
                )
                .unwrap();
            assert_eq!(
                rows.iter().map(|row| &row.id).collect::<Vec<_>>(),
                ids.iter().rev().take(8).collect::<Vec<_>>()
            );
            for row in &rows {
                let mut expected = store.load(&row.id).unwrap().unwrap();
                if !include_steps {
                    expected.steps.clear();
                }
                assert_eq!(
                    serde_json::to_value(row).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
            }
            pool.with_client(|client| {
                let mut counter = CountingReads { client, queries: 0 };
                let mut copies = rows.clone();
                load_history(&mut counter, &mut copies, include_steps, true)?;
                assert_eq!(counter.queries, 1 + usize::from(include_steps));
                assert_eq!(
                    serde_json::to_value(copies).unwrap(),
                    serde_json::to_value(&rows).unwrap()
                );
                load_history(&mut counter, &mut [], include_steps, true)?;
                assert_eq!(counter.queries, 1 + usize::from(include_steps));
                Ok(())
            })
            .unwrap();
        }
        let recovery_rows: Vec<_> = store
            .list(Some("Running"))
            .unwrap()
            .into_iter()
            .filter(|row| row.name == name)
            .collect();
        assert_eq!(recovery_rows.len(), ids.len());
        for row in recovery_rows {
            let mut expected = store.load(&row.id).unwrap().unwrap();
            expected.pending_events.clear();
            assert_eq!(
                serde_json::to_value(row).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
        pool.with_client(|client| {
            let mut missing: Vec<_> = (0..1001)
                .map(|i| test_workflow(format!("missing_{suffix}_{i}")))
                .collect();
            let mut counter = CountingReads { client, queries: 0 };
            load_history(&mut counter, &mut missing, true, true)?;
            assert_eq!(counter.queries, 4);
            assert!(missing
                .iter()
                .all(|workflow| workflow.steps.is_empty() && workflow.pending_events.is_empty()));
            counter.client.execute(
                "DELETE FROM _pylon_workflows WHERE id = ANY($1::text[])",
                &[&ids],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn status_counts_need_only_the_status_column() {
        let Some(pool) = test_pool() else {
            return;
        };
        pool.with_client(|client| {
            let mut tx = client.transaction()?;
            tx.batch_execute("CREATE TEMP TABLE _pylon_workflows (status TEXT NOT NULL) ON COMMIT DROP")?;
            assert_eq!(load_status_counts(&mut tx)?, WorkflowCounts::default());
            for (i, status) in ["Pending", "Running", "WaitingForEvent", "Sleeping", "Completed", "Failed", "Cancelled"].iter().enumerate() {
                tx.execute("INSERT INTO _pylon_workflows (status) SELECT $1 FROM generate_series(1, $2)", &[status, &((i + 1) as i32)])?;
            }
            assert_eq!(serde_json::to_value(load_status_counts(&mut tx)?).unwrap(), serde_json::json!({"pending":1,"running":2,"waiting":3,"sleeping":4,"completed":5,"failed":6,"cancelled":7}));
            tx.rollback()
        }).unwrap();
    }

    #[test]
    fn step_batches_bound_rows_and_payloads() {
        #[derive(Default)]
        struct BatchSizes(Vec<usize>);
        impl pylon_storage::pg_exec::PgConn for BatchSizes {
            fn execute(
                &mut self,
                _: &str,
                params: &[&(dyn postgres::types::ToSql + Sync)],
            ) -> Result<u64, postgres::Error> {
                let rows = params.len() / 11;
                self.0.push(rows);
                Ok(rows as u64)
            }
            fn query(
                &mut self,
                _: &str,
                _: &[&(dyn postgres::types::ToSql + Sync)],
            ) -> Result<Vec<postgres::Row>, postgres::Error> {
                panic!("unexpected read")
            }
            fn query_opt(
                &mut self,
                _: &str,
                _: &[&(dyn postgres::types::ToSql + Sync)],
            ) -> Result<Option<postgres::Row>, postgres::Error> {
                panic!("unexpected read")
            }
        }
        let step = test_workflow("batch".into()).steps.remove(0);
        let mut sizes = BatchSizes::default();
        insert_steps(&mut sizes, "batch", &[]).unwrap();
        assert!(sizes.0.is_empty());
        insert_steps(
            &mut sizes,
            "batch",
            &vec![step.clone(); STEP_BATCH_ROWS + 1],
        )
        .unwrap();
        assert_eq!(sizes.0, [STEP_BATCH_ROWS, 1]);
        sizes.0.clear();
        let mut large = step.clone();
        large.output = Some(serde_json::json!("x".repeat(STEP_BATCH_BYTES / 2)));
        insert_steps(&mut sizes, "batch", &[large.clone(), large]).unwrap();
        assert_eq!(sizes.0, [1, 1]);
        sizes.0.clear();
        let mut oversized = step.clone();
        oversized.output = Some(serde_json::json!("x".repeat(STEP_BATCH_BYTES * 2)));
        insert_steps(&mut sizes, "batch", &[step.clone(), oversized, step]).unwrap();
        assert_eq!(sizes.0, [1, 1, 1]);
    }

    #[test]
    fn step_batches_preserve_updates_and_rollback_later_failures() {
        let Some(pool) = test_pool() else {
            return;
        };
        let suffix = pylon_cluster::new_instance_id();
        let id = format!("wf_batch_{suffix}");
        let store = PgWorkflowStore::open(pool.clone(), suffix).unwrap();
        let mut workflow = test_workflow(id.clone());
        let template = workflow.steps[0].clone();
        workflow.steps = (0..STEP_BATCH_ROWS + 3)
            .map(|i| {
                let mut step = template.clone();
                step.step_id = format!("step-{i}");
                step.name = format!("step {i}");
                step.retry_count = i as u32;
                step.output = if i % 2 == 0 {
                    Some(serde_json::json!({"index": i}))
                } else {
                    None
                };
                step.error = if i % 3 == 0 {
                    Some("retry error".into())
                } else {
                    None
                };
                step.started_at = if i % 2 == 0 {
                    Some(format!("{i}Z"))
                } else {
                    None
                };
                step.completed_at = None;
                step.duration_ms = Some(i as u64);
                step
            })
            .collect();
        store.save(&workflow).unwrap();
        let original = store.load(&id).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&original).unwrap(),
            serde_json::to_value(&workflow).unwrap()
        );
        let mut bad = workflow.clone();
        bad.output = Some(serde_json::json!("must roll back"));
        bad.steps[STEP_BATCH_ROWS].name = "invalid\0text".into();
        assert!(store.save(&bad).is_err());
        assert_eq!(
            serde_json::to_value(store.load(&id).unwrap().unwrap()).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        workflow.steps[0].retry_count += 1;
        workflow.steps[0].output = Some(serde_json::json!({"retried": true}));
        workflow.steps.truncate(2);
        store.save(&workflow).unwrap();
        assert_eq!(
            serde_json::to_value(store.load(&id).unwrap().unwrap()).unwrap(),
            serde_json::to_value(&workflow).unwrap()
        );
        workflow.steps.clear();
        store.save(&workflow).unwrap();
        assert!(store.load(&id).unwrap().unwrap().steps.is_empty());
        pool.with_client(|client| {
            client.execute("DELETE FROM _pylon_workflows WHERE id=$1", &[&id])?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    #[ignore = "release-mode performance benchmark with local PostgreSQL"]
    fn benchmark_step_batches() {
        let Some(pool) = test_pool() else {
            return;
        };
        let suffix = pylon_cluster::new_instance_id();
        let id = format!("wf_bench_{suffix}");
        let store = PgWorkflowStore::open(pool.clone(), suffix).unwrap();
        let workflow = test_workflow(id.clone());
        store.save(&workflow).unwrap();
        let steps = vec![workflow.steps[0].clone(); 512];
        for batched in [false, true] {
            let mut samples = Vec::new();
            for _ in 0..7 {
                pool.with_client(|client| {
                    let mut tx = client.transaction()?;
                    tx.execute(
                        "DELETE FROM _pylon_workflow_steps WHERE workflow_id=$1",
                        &[&id],
                    )?;
                    let start = std::time::Instant::now();
                    if batched {
                        insert_steps(&mut tx, &id, &steps)?;
                    } else {
                        for (i, step) in steps.iter().enumerate() {
                            insert_step_batch(&mut tx, &id, i, std::slice::from_ref(step))?;
                        }
                    }
                    samples.push(start.elapsed().as_secs_f64() * 1000.0);
                    tx.rollback()
                })
                .unwrap();
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "512 workflow steps, batched={batched}: {:.3} ms median",
                samples[3]
            );
        }
        pool.with_client(|client| {
            client.execute("DELETE FROM _pylon_workflows WHERE id=$1", &[&id])?;
            Ok(())
        })
        .unwrap();
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

    #[test]
    fn cancel_and_events_work_without_the_lease_and_fence_the_driver() {
        let Some(pool) = test_pool() else {
            return;
        };
        let suffix = pylon_cluster::new_instance_id();
        let store = PgWorkflowStore::open(Arc::clone(&pool), format!("drv_{suffix}")).unwrap();
        let id = format!("wf_fence_{suffix}");
        let mut workflow = test_workflow(id.clone());
        workflow.status = WorkflowStatus::WaitingForEvent;
        workflow.waiting_for = Some("reply".into());
        workflow.wait_deadline = Some(now_secs_i64() as u64 + 3600);
        store.save(&workflow).unwrap();

        // A driver holds the lease (a step is running elsewhere).
        let token = format!("tok_{suffix}");
        assert!(store.try_acquire(&id, &token, 30).unwrap());

        // Events still land, and the waiting run reports delivered.
        let d = store
            .push_event(&id, "reply", &serde_json::json!({"body": "yes"}), 100)
            .unwrap();
        assert!(d.delivered && !d.buffered);
        let d = store
            .push_event(&id, "other", &serde_json::json!({}), 100)
            .unwrap();
        assert!(!d.delivered && d.buffered);
        let events = store.load_events(&id).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event, "reply");
        assert!(store.has_due_work(&id, 0).unwrap());
        assert!(store.due_wait_ids(0).unwrap().contains(&id));

        // Consuming an event deletes exactly that row in the save tx.
        let mut consumed = store.load(&id).unwrap().unwrap();
        consumed.consumed_events = vec![events[0].seq];
        consumed.status = WorkflowStatus::Running;
        consumed.waiting_for = None;
        store.save_owned(&consumed, &token).unwrap();
        let left = store.load_events(&id).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].event, "other");

        // Cancel succeeds while the lease is held...
        assert!(store.cancel(&id, Some("STOP"), 5).unwrap());
        assert!(!store.cancel(&id, Some("again"), 6).unwrap());
        // ...and the lease holder can no longer overwrite it.
        let mut late = consumed.clone();
        late.status = WorkflowStatus::Running;
        assert!(store.save_owned(&late, &token).is_err());
        assert!(store.save(&late).is_err());
        let after = store.load(&id).unwrap().unwrap();
        assert_eq!(after.status, WorkflowStatus::Cancelled);
        assert_eq!(after.cancel_reason.as_deref(), Some("STOP"));
        // A cancelled run rejects events.
        assert!(store
            .push_event(&id, "reply", &serde_json::json!({}), 100)
            .is_err());
        assert!(store.cancel(&format!("missing_{suffix}"), None, 1).is_err());

        pool.with_client(|client| {
            client.execute("DELETE FROM _pylon_workflows WHERE id=$1", &[&id])?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn insert_new_dedupes_active_runs_by_name_and_key() {
        let Some(pool) = test_pool() else {
            return;
        };
        let suffix = pylon_cluster::new_instance_id();
        let store = PgWorkflowStore::open(Arc::clone(&pool), format!("key_{suffix}")).unwrap();
        let key = format!("lead_{suffix}");
        let mut a = test_workflow(format!("wf_a_{suffix}"));
        a.status = WorkflowStatus::Pending;
        a.key = Some(key.clone());
        assert_eq!(store.insert_new(&a).unwrap(), None);
        let mut b = a.clone();
        b.id = format!("wf_b_{suffix}");
        assert_eq!(store.insert_new(&b).unwrap(), Some(a.id.clone()));

        let listed = store
            .list_filtered(
                &WorkflowFilter {
                    key: Some(key.clone()),
                    status: Some(StatusFilter::Active),
                    ..Default::default()
                },
                10,
                false,
            )
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, a.id);
        assert!(listed[0].steps.is_empty());

        store.cancel(&a.id, None, 1).unwrap();
        assert_eq!(store.insert_new(&b).unwrap(), None);
        assert_eq!(store.load(&b.id).unwrap().unwrap().key, Some(key.clone()));

        pool.with_client(|client| {
            client.execute(
                "DELETE FROM _pylon_workflows WHERE id = ANY($1)",
                &[&vec![a.id.clone(), b.id.clone()]],
            )?;
            Ok(())
        })
        .unwrap();
    }
}
