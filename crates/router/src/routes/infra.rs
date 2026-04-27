//! Operator-facing infra endpoints: cache, pubsub, jobs, scheduler.
//!
//! Every route is admin-gated. These are raw operator surfaces with
//! no per-key/per-channel/per-job authz model — apps that want to
//! expose any of this to end users wrap it in a server function with
//! the right policy.

use crate::{json_error, json_error_safe, require_admin, RouterContext};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    // ---- Cache ----
    if url == "/api/cache" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        return Some(ctx.cache.handle_command(body));
    }

    if let Some(cache_key) = url.strip_prefix("/api/cache/") {
        let cache_key = cache_key.split('?').next().unwrap_or(cache_key);
        if method == HttpMethod::Get && !cache_key.is_empty() {
            if let Some(err) = require_admin(ctx) {
                return Some(err);
            }
            return Some(ctx.cache.handle_get(cache_key));
        }
        if method == HttpMethod::Delete && !cache_key.is_empty() {
            if let Some(err) = require_admin(ctx) {
                return Some(err);
            }
            return Some(ctx.cache.handle_delete(cache_key));
        }
    }

    // ---- Pub/Sub ----
    if url == "/api/pubsub/publish" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        return Some(ctx.pubsub.handle_publish(body));
    }

    if url == "/api/pubsub/channels" && method == HttpMethod::Get {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        return Some(ctx.pubsub.handle_channels());
    }

    if let Some(channel_name) = url.strip_prefix("/api/pubsub/history/") {
        let channel_name = channel_name.split('?').next().unwrap_or(channel_name);
        if method == HttpMethod::Get && !channel_name.is_empty() {
            if let Some(err) = require_admin(ctx) {
                return Some(err);
            }
            return Some(ctx.pubsub.handle_history(channel_name, url));
        }
    }

    // ---- Jobs ----
    if url == "/api/jobs" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some((
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                ));
            }
        };
        let name = match data.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return Some((400, json_error("MISSING_NAME", "name is required"))),
        };
        let payload = data
            .get("payload")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let priority = data
            .get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("normal");
        let delay = data.get("delay_secs").and_then(|v| v.as_u64()).unwrap_or(0);
        let max_retries = data
            .get("max_retries")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as u32;
        let queue = data
            .get("queue")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let id = ctx
            .jobs
            .enqueue(name, payload, priority, delay, max_retries, queue);
        return Some((
            201,
            serde_json::json!({"id": id, "status": "pending"}).to_string(),
        ));
    }

    if url == "/api/jobs/stats" && method == HttpMethod::Get {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        let stats = ctx.jobs.stats();
        return Some((
            200,
            serde_json::to_string(&stats).unwrap_or_else(|_| "{}".into()),
        ));
    }

    if url == "/api/jobs/dead" && method == HttpMethod::Get {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        let dead = ctx.jobs.dead_letters();
        return Some((
            200,
            serde_json::to_string(&dead).unwrap_or_else(|_| "[]".into()),
        ));
    }

    if let Some(rest) = url.strip_prefix("/api/jobs/dead/") {
        let rest = rest.split('?').next().unwrap_or(rest);
        if let Some(job_id) = rest.strip_suffix("/retry") {
            if method == HttpMethod::Post && !job_id.is_empty() {
                if let Some(err) = require_admin(ctx) {
                    return Some(err);
                }
                if ctx.jobs.retry_dead(job_id) {
                    return Some((
                        200,
                        serde_json::json!({"retried": true, "id": job_id}).to_string(),
                    ));
                }
                return Some((
                    404,
                    json_error("NOT_FOUND", "Job not found in dead letter queue"),
                ));
            }
        }
    }

    if url.starts_with("/api/jobs") && method == HttpMethod::Get {
        let path = url.split('?').next().unwrap_or(url);
        if path == "/api/jobs" {
            if let Some(err) = require_admin(ctx) {
                return Some(err);
            }
            let status_filter = url
                .split("status=")
                .nth(1)
                .and_then(|s| s.split('&').next());
            let queue_filter = url.split("queue=").nth(1).and_then(|s| s.split('&').next());
            let limit: usize = url
                .split("limit=")
                .nth(1)
                .and_then(|s| s.split('&').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(50)
                .min(200);
            let jobs = ctx.jobs.list_jobs(status_filter, queue_filter, limit);
            return Some((
                200,
                serde_json::to_string(&jobs).unwrap_or_else(|_| "[]".into()),
            ));
        }
    }

    if let Some(job_id) = url.strip_prefix("/api/jobs/") {
        let job_id = job_id.split('?').next().unwrap_or(job_id);
        if method == HttpMethod::Get && !job_id.is_empty() && job_id != "stats" && job_id != "dead"
        {
            if let Some(err) = require_admin(ctx) {
                return Some(err);
            }
            if let Some(job) = ctx.jobs.get_job(job_id) {
                return Some((
                    200,
                    serde_json::to_string(&job).unwrap_or_else(|_| "{}".into()),
                ));
            }
            return Some((
                404,
                json_error("NOT_FOUND", &format!("Job {job_id} not found")),
            ));
        }
    }

    // ---- Scheduler ----
    if url == "/api/scheduler" && method == HttpMethod::Get {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        let tasks = ctx.scheduler.list_tasks();
        return Some((
            200,
            serde_json::to_string(&tasks).unwrap_or_else(|_| "[]".into()),
        ));
    }

    if let Some(task_name) = url.strip_prefix("/api/scheduler/trigger/") {
        let task_name = task_name.split('?').next().unwrap_or(task_name);
        if method == HttpMethod::Post && !task_name.is_empty() {
            if let Some(err) = require_admin(ctx) {
                return Some(err);
            }
            if ctx.scheduler.trigger(task_name) {
                return Some((
                    200,
                    serde_json::json!({"triggered": true, "task": task_name}).to_string(),
                ));
            }
            return Some((
                404,
                json_error(
                    "NOT_FOUND",
                    &format!("Scheduled task \"{task_name}\" not found"),
                ),
            ));
        }
    }

    None
}
