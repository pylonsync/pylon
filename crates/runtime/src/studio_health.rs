//! The health report for Studio: `GET /admin/health`.
//!
//! One answer with the checks an operator runs when something is wrong: can
//! the server reach the database and how fast, does the functions runtime
//! answer, do jobs wait past their time, does this machine run the cron tick,
//! how many realtime clients are connected, how much memory is in use, and
//! does the server take traffic.
//!
//! The server does not measure disk space, so the report has none.

use std::time::{Duration, Instant};

use crate::jobs::{Job, JobStatus};

/// The result of one probe: whether it passed, how long it took, and why it
/// failed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Probe {
    pub ok: bool,
    pub latency_ms: f64,
    pub error: Option<String>,
}

impl Probe {
    /// Run `check` and time it.
    pub(crate) fn run(check: impl FnOnce() -> Result<(), String>) -> Self {
        let started = Instant::now();
        let result = check();
        Self {
            ok: result.is_ok(),
            latency_ms: millis(started.elapsed()),
            error: result.err(),
        }
    }

    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ok": self.ok,
            "latency_ms": self.latency_ms,
            "error": self.error,
        })
    }
}

fn millis(d: Duration) -> f64 {
    (d.as_secs_f64() * 100_000.0).round() / 100.0
}

/// A job time: Unix seconds, written as `"1795114363Z"`.
fn job_time(s: &str) -> Option<u64> {
    s.trim_end_matches('Z').parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobQueue;

    #[test]
    fn a_probe_reports_the_failure_and_a_time() {
        let ok = Probe::run(|| Ok(()));
        assert!(ok.ok && ok.error.is_none() && ok.latency_ms >= 0.0);
        let bad = Probe::run(|| Err("database: connection refused".into()));
        assert!(!bad.ok);
        assert_eq!(bad.to_json()["error"], "database: connection refused");
    }

    #[test]
    fn job_lag_is_the_longest_wait_past_the_due_time() {
        let queue = JobQueue::new(10);
        queue.enqueue("a", serde_json::json!({}));
        queue.enqueue_with_options(
            "later",
            serde_json::json!({}),
            crate::jobs::Priority::Normal,
            3600,
            0,
            "default",
        );
        let jobs = queue.list_jobs(Some("pending"), None, 10);
        assert_eq!(jobs.len(), 2);
        let created = job_time(&jobs[0].created_at).unwrap();

        // The job with no delay is 30 seconds overdue. The delayed job is
        // not due, so it does not count.
        assert_eq!(queue.oldest_due_lag_secs(created + 30), Some(30));
        // Past the delay, the first job is still the most overdue.
        assert_eq!(queue.oldest_due_lag_secs(created + 3700), Some(3700));
        assert_eq!(JobQueue::new(10).oldest_due_lag_secs(created), None);
    }

    #[test]
    fn job_lag_counts_a_job_past_the_first_two_hundred() {
        let queue = JobQueue::new(1000);
        // The oldest job is due first. Three hundred newer jobs wait an hour,
        // so they are not due.
        queue.enqueue("oldest", serde_json::json!({}));
        for _ in 0..300 {
            queue.enqueue_with_options(
                "later",
                serde_json::json!({}),
                crate::jobs::Priority::Normal,
                3600,
                0,
                "default",
            );
        }
        let created = job_time(&queue.list_jobs(Some("pending"), None, 1)[0].created_at).unwrap();
        assert_eq!(queue.oldest_due_lag_secs(created + 60), Some(60));
    }

    #[test]
    fn job_times_parse_with_and_without_the_suffix() {
        assert_eq!(job_time("1795114363Z"), Some(1_795_114_363));
        assert_eq!(job_time("1795114363"), Some(1_795_114_363));
        assert_eq!(job_time("soon"), None);
    }
}
