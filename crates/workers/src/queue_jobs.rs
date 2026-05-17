//! `WorkersJobs` — `JobOps` adapter backed by Cloudflare Queues
//! for enqueue + a KV-indexed registry for the list/stats/DLQ
//! surface that Queues doesn't expose natively.
//!
//! Why split storage: Cloudflare Queues handles the actual
//! delivery + retry semantics (`max_retries` on send, automatic
//! retry-with-delay, DLQ-on-exhaust). What it does NOT expose is:
//!
//! - Iterating queued messages (no `list_messages` API)
//! - Per-message status query
//! - Dead-letter inspection
//! - Stats counters
//!
//! The framework's `JobOps` trait wants all of that for the
//! Studio dashboard. We satisfy the surface by writing a small
//! registry record to KV alongside every enqueue — `jobs:<id>`
//! key holding the JSON envelope + status. The queue consumer
//! (Workers fetch handler in queue-event mode) updates the
//! record's status as it processes. List/stats walk the KV
//! prefix.

#![cfg(feature = "workers")]

use pylon_router::JobOps;
use worker::{kv::KvStore, Env, Queue};

const REGISTRY_PREFIX: &str = "jobs:";
const DLQ_PREFIX: &str = "jobs_dlq:";

/// Adapter. Holds the Queue producer binding for enqueues + a KV
/// binding for the registry side. The wrangler.toml declares
/// both:
///
/// ```toml
/// [[queues.producers]]
/// binding = "PYLON_JOBS"
/// queue = "pylon-jobs"
/// [[kv_namespaces]]
/// binding = "PYLON_JOBS_REGISTRY"
/// id = "..."
/// ```
pub struct WorkersJobs {
    queue: Queue,
    registry: KvStore,
}

// SAFETY: same rationale as KvCache + WorkersRooms — Workers'
// single-threaded runtime never crosses these between threads.
unsafe impl Send for WorkersJobs {}
unsafe impl Sync for WorkersJobs {}

impl WorkersJobs {
    pub fn new(queue: Queue, registry: KvStore) -> Self {
        Self { queue, registry }
    }

    /// Try to construct from env bindings. Returns None if either
    /// binding is missing — caller falls back to NoopAll's typed
    /// QUEUES_BINDING_REQUIRED 503s.
    pub fn from_env(env: &Env) -> Option<Self> {
        let queue = env.queue("PYLON_JOBS").ok()?;
        let registry = env.kv("PYLON_JOBS_REGISTRY").ok()?;
        Some(Self::new(queue, registry))
    }
}

impl JobOps for WorkersJobs {
    fn enqueue(
        &self,
        name: &str,
        payload: serde_json::Value,
        priority: &str,
        delay_secs: u64,
        max_retries: u32,
        queue: &str,
    ) -> String {
        let id = format!("job_{}", random_hex(16));
        let envelope = serde_json::json!({
            "id": id,
            "name": name,
            "payload": payload,
            "priority": priority,
            "queue": queue,
            "max_retries": max_retries,
            "delay_secs": delay_secs,
            "status": "queued",
            "created_at": now_ms(),
        });
        // 1. Persist the registry record so list/get/stats see it
        //    immediately (Queues' eventual-consistency window
        //    would otherwise hide just-enqueued jobs from the
        //    dashboard for a second or two).
        let key = format!("{REGISTRY_PREFIX}{id}");
        let envelope_str = envelope.to_string();
        let put_fut = match self.registry.put(&key, envelope_str.clone()) {
            Ok(b) => b.execute(),
            Err(_) => return String::new(),
        };
        let _ = futures::executor::block_on(put_fut);
        // 2. Send to the Cloudflare Queue. Queue's send takes a
        //    serializable payload + optional delay; the registry
        //    id is embedded so the consumer can look up + update.
        let send_msg = worker::MessageBuilder::new(envelope.clone())
            .delay_seconds(u32::try_from(delay_secs).unwrap_or(u32::MAX))
            .build();
        let _ = futures::executor::block_on(self.queue.send(send_msg)).map_err(|e| e.to_string());
        id
    }

    fn stats(&self) -> serde_json::Value {
        // Walk the registry prefix to count by status. KV list is
        // paginated; for the dashboard surface we just take the
        // first 1000 — sufficient for any non-pathological queue
        // depth, and Studio doesn't paginate the stats card
        // anyway.
        let scan = match futures::executor::block_on(
            self.registry
                .list()
                .prefix(REGISTRY_PREFIX.into())
                .limit(1000)
                .execute(),
        ) {
            Ok(s) => s,
            Err(_) => return serde_json::json!({}),
        };
        let mut pending = 0u64;
        let mut running = 0u64;
        let mut completed = 0u64;
        let mut failed = 0u64;
        let mut dead = 0u64;
        for k in scan.keys {
            let Ok(Some(raw)) = futures::executor::block_on(self.registry.get(&k.name).text())
            else {
                continue;
            };
            let Ok(j): std::result::Result<serde_json::Value, _> = serde_json::from_str(&raw)
            else {
                continue;
            };
            match j.get("status").and_then(|s| s.as_str()).unwrap_or("") {
                "queued" => pending += 1,
                "running" => running += 1,
                "completed" => completed += 1,
                "failed" => failed += 1,
                "dead" => dead += 1,
                _ => {}
            }
        }
        serde_json::json!({
            "pending": pending,
            "running": running,
            "completed": completed,
            "failed": failed,
            "dead": dead,
            "handlers": serde_json::Value::Null,
        })
    }

    fn dead_letters(&self) -> serde_json::Value {
        let scan = match futures::executor::block_on(
            self.registry
                .list()
                .prefix(DLQ_PREFIX.into())
                .limit(100)
                .execute(),
        ) {
            Ok(s) => s,
            Err(_) => return serde_json::json!([]),
        };
        let mut out: Vec<serde_json::Value> = Vec::new();
        for k in scan.keys {
            if let Ok(Some(raw)) = futures::executor::block_on(self.registry.get(&k.name).text()) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                    out.push(v);
                }
            }
        }
        serde_json::Value::Array(out)
    }

    fn retry_dead(&self, id: &str) -> bool {
        let dlq_key = format!("{DLQ_PREFIX}{id}");
        let registry_key = format!("{REGISTRY_PREFIX}{id}");
        let Ok(Some(raw)) = futures::executor::block_on(self.registry.get(&dlq_key).text()) else {
            return false;
        };
        let Ok(envelope): std::result::Result<serde_json::Value, _> = serde_json::from_str(&raw)
        else {
            return false;
        };
        // Re-enqueue: send to the Queue + put back into the active
        // registry under its old id.
        let send_msg = worker::MessageBuilder::new(envelope.clone()).build();
        if futures::executor::block_on(self.queue.send(send_msg)).is_err() {
            return false;
        }
        if let Ok(b) = self.registry.put(&registry_key, raw) {
            let _ = futures::executor::block_on(b.execute());
        }
        let _ = futures::executor::block_on(self.registry.delete(&dlq_key));
        true
    }

    fn list_jobs(
        &self,
        status: Option<&str>,
        queue: Option<&str>,
        limit: usize,
    ) -> serde_json::Value {
        let scan = match futures::executor::block_on(
            self.registry
                .list()
                .prefix(REGISTRY_PREFIX.into())
                .limit(limit.min(1000) as u64)
                .execute(),
        ) {
            Ok(s) => s,
            Err(_) => return serde_json::json!([]),
        };
        let mut out: Vec<serde_json::Value> = Vec::new();
        for k in scan.keys {
            let Ok(Some(raw)) = futures::executor::block_on(self.registry.get(&k.name).text())
            else {
                continue;
            };
            let Ok(j): std::result::Result<serde_json::Value, _> = serde_json::from_str(&raw)
            else {
                continue;
            };
            if let Some(want_status) = status {
                if j.get("status").and_then(|s| s.as_str()) != Some(want_status) {
                    continue;
                }
            }
            if let Some(want_queue) = queue {
                if j.get("queue").and_then(|s| s.as_str()) != Some(want_queue) {
                    continue;
                }
            }
            out.push(j);
        }
        serde_json::Value::Array(out)
    }

    fn get_job(&self, id: &str) -> Option<serde_json::Value> {
        let key = format!("{REGISTRY_PREFIX}{id}");
        let raw = futures::executor::block_on(self.registry.get(&key).text())
            .ok()
            .flatten()?;
        serde_json::from_str(&raw).ok()
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn random_hex(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}
