//! Cron-based scheduler that enqueues jobs on a recurring schedule.
//!
//! The scheduler maintains a list of named tasks, each with a cron expression
//! and an associated job handler. A background thread wakes every 30 seconds
//! and enqueues jobs for any tasks whose cron expression matches the current
//! minute (deduplicating within the same minute).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;

use crate::cron::CronExpr;
use crate::jobs::{JobHandler, JobQueue};
use crate::leader::{AlwaysLeader, Leadership};

// ---------------------------------------------------------------------------
// Scheduled task
// ---------------------------------------------------------------------------

/// Internal representation of a recurring scheduled task.
#[allow(dead_code)]
struct ScheduledTask {
    name: String,
    cron: CronExpr,
    /// The cron expression as it was registered, for display.
    schedule: String,
    handler: JobHandler,
    enabled: bool,
    /// Unix timestamp of the last time this task was enqueued.
    last_run: Option<u64>,
}

// ---------------------------------------------------------------------------
// Public task info
// ---------------------------------------------------------------------------

/// Read-only information about a scheduled task, returned by `list_tasks()`.
#[derive(Debug, Clone, Serialize)]
pub struct TaskInfo {
    pub name: String,
    pub enabled: bool,
    pub last_run: Option<u64>,
    /// The cron expression of the task.
    pub schedule: String,
    /// Unix time of the next minute the expression matches. `None` when the
    /// task is disabled or nothing matches in the next [`NEXT_RUN_HORIZON_DAYS`].
    pub next_run: Option<u64>,
}

/// How far ahead `list_tasks` looks for the next run.
const NEXT_RUN_HORIZON_DAYS: u64 = 8;

/// The start of the first minute after `now` that `cron` matches.
fn next_run_after(cron: &CronExpr, now: u64) -> Option<u64> {
    let first = (now / 60 + 1) * 60;
    (0..NEXT_RUN_HORIZON_DAYS * 24 * 60)
        .map(|i| first + i * 60)
        .find(|t| cron.matches(*t))
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// The cron scheduler. Runs registered tasks on their configured schedules.
pub struct Scheduler {
    tasks: Mutex<Vec<ScheduledTask>>,
    job_queue: Arc<JobQueue>,
    running: AtomicBool,
    /// Cross-machine singleton gate for the cron tick. Single-machine
    /// deploys use [`AlwaysLeader`]; multi-machine Postgres deploys use
    /// the advisory-lock leader so each cron match enqueues ONCE
    /// cluster-wide, not once per machine.
    leadership: Arc<dyn Leadership>,
}

impl Scheduler {
    pub fn new(job_queue: Arc<JobQueue>) -> Self {
        Self::with_leadership(job_queue, Arc::new(AlwaysLeader))
    }

    pub fn with_leadership(job_queue: Arc<JobQueue>, leadership: Arc<dyn Leadership>) -> Self {
        Self {
            tasks: Mutex::new(Vec::new()),
            job_queue,
            running: AtomicBool::new(true),
            leadership,
        }
    }

    /// Register a cron task. The handler will also be registered with the job
    /// queue so that workers can execute it.
    pub fn schedule(&self, name: &str, cron_expr: &str, handler: JobHandler) -> Result<(), String> {
        let cron = CronExpr::parse(cron_expr)?;

        // Register handler with job queue so workers can pick it up.
        self.job_queue.register(name, Arc::clone(&handler));

        self.tasks.lock().unwrap().push(ScheduledTask {
            name: name.to_string(),
            cron,
            schedule: cron_expr.trim().to_string(),
            handler,
            enabled: true,
            last_run: None,
        });

        Ok(())
    }

    /// Start the scheduler loop in a background thread.
    pub fn start(self: Arc<Self>) -> SchedulerHandle {
        let scheduler = Arc::clone(&self);
        let handle = std::thread::spawn(move || {
            while scheduler.running.load(Ordering::Relaxed) {
                scheduler.tick();
                // Sleep in short intervals so we can observe the shutdown flag
                // without waiting a full 30 seconds.
                for _ in 0..30 {
                    if !scheduler.running.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        });
        SchedulerHandle {
            scheduler: self,
            handle: Some(handle),
        }
    }

    /// Check all tasks and enqueue any that match the current time.
    ///
    /// This is also useful for testing: call `tick()` directly to simulate
    /// the scheduler loop without waiting for the background thread.
    pub fn tick(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.tick_at(now);
    }

    /// Internal tick with an explicit timestamp (for testability).
    fn tick_at(&self, now: u64) {
        // Followers keep their task list warm but never enqueue — cron
        // fires exactly once cluster-wide, on the leader. last_run is
        // deliberately NOT advanced on followers: a machine promoted
        // mid-minute may enqueue a task the old leader already fired
        // that minute (at-least-once during failover) — acceptable for
        // cron; the failure mode we refuse is N-times-per-minute
        // steady-state duplication.
        if !self.leadership.is_leader() {
            return;
        }
        let current_minute = now / 60;

        let mut tasks = self.tasks.lock().unwrap();
        for task in tasks.iter_mut() {
            if !task.enabled {
                continue;
            }

            let last_minute = task.last_run.map(|t| t / 60).unwrap_or(0);
            if current_minute > last_minute && task.cron.matches(now) {
                task.last_run = Some(now);
                self.job_queue.enqueue(
                    &task.name,
                    serde_json::json!({
                        "scheduled": true,
                        "timestamp": now,
                    }),
                );
            }
        }
    }

    /// List all scheduled tasks.
    pub fn list_tasks(&self) -> Vec<TaskInfo> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.tasks
            .lock()
            .unwrap()
            .iter()
            .map(|t| TaskInfo {
                name: t.name.clone(),
                enabled: t.enabled,
                last_run: t.last_run,
                schedule: t.schedule.clone(),
                next_run: t.enabled.then(|| next_run_after(&t.cron, now)).flatten(),
            })
            .collect()
    }

    /// Whether this machine runs the cron tick. Always true on one machine.
    /// With several machines on Postgres, one of them holds the lock.
    pub fn is_leader(&self) -> bool {
        self.leadership.is_leader()
    }

    /// Enable or disable a task by name. Returns true if the task was found.
    pub fn set_enabled(&self, name: &str, enabled: bool) -> bool {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.iter_mut().find(|t| t.name == name) {
            task.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Manually trigger a scheduled task by enqueueing it immediately.
    /// Returns true if the task was found and enqueued.
    pub fn trigger(&self, name: &str) -> bool {
        let tasks = self.tasks.lock().unwrap();
        if tasks.iter().any(|t| t.name == name) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            drop(tasks);
            self.job_queue.enqueue(
                name,
                serde_json::json!({
                    "scheduled": true,
                    "manual_trigger": true,
                    "timestamp": now,
                }),
            );
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// SchedulerHandle
// ---------------------------------------------------------------------------

/// Handle returned by `Scheduler::start()` to stop the background thread.
pub struct SchedulerHandle {
    scheduler: Arc<Scheduler>,
    #[allow(dead_code)]
    handle: Option<std::thread::JoinHandle<()>>,
}

impl SchedulerHandle {
    /// Signal the scheduler to stop after its current sleep cycle.
    pub fn stop(&self) {
        self.scheduler.running.store(false, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobResult;

    #[test]
    fn list_tasks_reports_the_schedule_and_the_next_run() {
        let q = Arc::new(JobQueue::new(100));
        let sched = Scheduler::new(Arc::clone(&q));
        sched
            .schedule("cleanup", " */5 * * * * ", Arc::new(|_| JobResult::Success))
            .unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let task = &sched.list_tasks()[0];
        assert_eq!(task.schedule, "*/5 * * * *");
        let next = task.next_run.expect("a next run");
        assert!(next > now && next <= now + 5 * 60, "{next} vs {now}");
        assert_eq!(next % 300, 0);

        sched.set_enabled("cleanup", false);
        assert_eq!(sched.list_tasks()[0].next_run, None);
    }

    #[test]
    fn next_run_is_none_when_nothing_matches_in_the_horizon() {
        // 30 February never comes.
        let cron = CronExpr::parse("0 0 30 2 *").unwrap();
        assert_eq!(next_run_after(&cron, 1_795_000_000), None);
    }

    #[test]
    fn schedule_registers_handler() {
        let q = Arc::new(JobQueue::new(100));
        let sched = Scheduler::new(Arc::clone(&q));

        sched
            .schedule("cleanup", "*/5 * * * *", Arc::new(|_| JobResult::Success))
            .unwrap();

        let tasks = sched.list_tasks();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "cleanup");
        assert!(tasks[0].enabled);
        assert!(tasks[0].last_run.is_none());

        // Handler should be registered with the queue.
        let stats = q.stats();
        assert!(stats.handlers.contains(&"cleanup".to_string()));
    }

    #[test]
    fn scheduled_handler_actually_runs_via_queue() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Regression: the server used to `job_queue.register(name, real)` and
        // THEN `scheduler.schedule(name, cron, no_op)`, but schedule() also
        // registers (last-write-wins) — so the no-op clobbered the real
        // handler and the cleanup never ran. Passing the real handler to
        // schedule() must make it the one the queue executes.
        let q = Arc::new(JobQueue::new(100));
        let sched = Scheduler::new(Arc::clone(&q));
        let runs = Arc::new(AtomicUsize::new(0));
        let runs2 = Arc::clone(&runs);
        sched
            .schedule(
                "cleanup",
                "* * * * *",
                Arc::new(move |_| {
                    runs2.fetch_add(1, Ordering::SeqCst);
                    JobResult::Success
                }),
            )
            .unwrap();
        q.enqueue("cleanup", serde_json::json!({}));
        assert!(q.process_one(), "a job should have been processed");
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "the handler passed to schedule() must run, not a no-op"
        );
    }

    #[test]
    fn schedule_rejects_bad_cron() {
        let q = Arc::new(JobQueue::new(100));
        let sched = Scheduler::new(Arc::clone(&q));

        let result = sched.schedule("bad", "not a cron", Arc::new(|_| JobResult::Success));
        assert!(result.is_err());
    }

    #[test]
    fn tick_enqueues_matching_tasks() {
        let q = Arc::new(JobQueue::new(100));
        let sched = Scheduler::new(Arc::clone(&q));

        // Every minute.
        sched
            .schedule("every_min", "* * * * *", Arc::new(|_| JobResult::Success))
            .unwrap();

        // A specific timestamp that matches "* * * * *".
        sched.tick_at(1705314600); // 2024-01-15 10:30 UTC

        assert_eq!(q.pending_count(), 1);
    }

    #[test]
    fn tick_deduplicates_within_same_minute() {
        let q = Arc::new(JobQueue::new(100));
        let sched = Scheduler::new(Arc::clone(&q));

        sched
            .schedule("dedup", "* * * * *", Arc::new(|_| JobResult::Success))
            .unwrap();

        // Tick at :30 seconds.
        sched.tick_at(1705314600);
        // Tick again at :45 seconds (same minute).
        sched.tick_at(1705314615);

        // Should only have enqueued once.
        assert_eq!(q.pending_count(), 1);
    }

    #[test]
    fn tick_enqueues_again_next_minute() {
        let q = Arc::new(JobQueue::new(100));
        let sched = Scheduler::new(Arc::clone(&q));

        sched
            .schedule("repeat", "* * * * *", Arc::new(|_| JobResult::Success))
            .unwrap();

        sched.tick_at(1705314600);
        sched.tick_at(1705314660); // next minute

        assert_eq!(q.pending_count(), 2);
    }

    #[test]
    fn tick_skips_disabled_tasks() {
        let q = Arc::new(JobQueue::new(100));
        let sched = Scheduler::new(Arc::clone(&q));

        sched
            .schedule("disabled", "* * * * *", Arc::new(|_| JobResult::Success))
            .unwrap();
        sched.set_enabled("disabled", false);

        sched.tick_at(1705314600);
        assert_eq!(q.pending_count(), 0);
    }

    #[test]
    fn set_enabled_returns_false_for_unknown() {
        let q = Arc::new(JobQueue::new(100));
        let sched = Scheduler::new(Arc::clone(&q));
        assert!(!sched.set_enabled("nonexistent", false));
    }

    #[test]
    fn trigger_enqueues_immediately() {
        let q = Arc::new(JobQueue::new(100));
        let sched = Scheduler::new(Arc::clone(&q));

        sched
            .schedule("manual", "0 0 1 1 *", Arc::new(|_| JobResult::Success))
            .unwrap();

        assert!(sched.trigger("manual"));
        assert_eq!(q.pending_count(), 1);
    }

    #[test]
    fn trigger_returns_false_for_unknown() {
        let q = Arc::new(JobQueue::new(100));
        let sched = Scheduler::new(Arc::clone(&q));
        assert!(!sched.trigger("nonexistent"));
    }

    #[test]
    fn tick_does_not_match_wrong_schedule() {
        let q = Arc::new(JobQueue::new(100));
        let sched = Scheduler::new(Arc::clone(&q));

        // Only at midnight on January 1st.
        sched
            .schedule("yearly", "0 0 1 1 *", Arc::new(|_| JobResult::Success))
            .unwrap();

        // 2024-01-15 10:30 should NOT match.
        sched.tick_at(1705314600);
        assert_eq!(q.pending_count(), 0);
    }
}
