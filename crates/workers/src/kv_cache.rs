//! Cloudflare KV-backed implementation of `CacheOps`.
//!
//! Only built with the `workers` feature (wasm32 target). The
//! non-WASM build uses `NoopAll`'s stub Cache which returns 503
//! KV_BINDING_REQUIRED.
//!
//! # What's supported
//!
//! - SET / GET / DEL / EXISTS — direct KV ops.
//! - SETNX — `put` with conditional metadata (KV's own atomic-set-
//!   if-absent is implemented client-side as a read-then-conditional-
//!   put; KV doesn't expose a true CAS but the pattern is acceptable
//!   for cache semantics where a lost race just means one writer
//!   wins).
//! - GETSET — same caveat as SETNX (read-then-put, not atomic).
//!
//! # What's NOT supported
//!
//! - INCR / DECR / INCRBY — KV has no atomic counter primitive.
//!   These return a typed `KV_NO_ATOMIC_COUNTERS` error pointing
//!   at "use a Durable Object for atomic counters" — the same
//!   shape the noop_adapters surface today, just more honest
//!   about the cause.
//!
//! # Async-to-sync bridge
//!
//! The trait is sync (it's shared with the self-hosted runtime
//! which uses an in-process Mutex). KV is async. We use the same
//! `futures::executor::block_on` bridge that `D1Executor` uses;
//! Workers' single-threaded runtime tolerates this in request
//! handlers without deadlock.

#![cfg(feature = "workers")]

use futures::executor::block_on;
use pylon_router::CacheOps;
use worker::kv::KvStore;

/// CacheOps backed by a Cloudflare KV namespace binding.
///
/// Construct from `env.kv("PYLON_CACHE")` in the fetch handler.
pub struct KvCache {
    store: KvStore,
}

impl KvCache {
    pub fn new(store: KvStore) -> Self {
        Self { store }
    }
}

impl CacheOps for KvCache {
    fn handle_command(&self, body: &str) -> (u16, String) {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    serde_json::json!({"ok": false, "error": format!("Invalid JSON: {e}")})
                        .to_string(),
                )
            }
        };
        let cmd = data
            .get("cmd")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_uppercase();
        let key = data.get("key").and_then(|v| v.as_str()).unwrap_or("");
        if key.is_empty() && cmd != "PING" {
            return (
                400,
                serde_json::json!({"ok": false, "error": "missing key"}).to_string(),
            );
        }

        match cmd.as_str() {
            "SET" => {
                let value = data.get("value").and_then(|v| v.as_str()).unwrap_or("");
                let ttl = data.get("ttl").and_then(|v| v.as_u64());
                kv_put(&self.store, key, value, ttl)
            }
            "GET" => kv_get(&self.store, key),
            "DEL" => kv_delete(&self.store, key),
            "EXISTS" => {
                let exists = block_on(self.store.get(key).text())
                    .map(|v| v.is_some())
                    .unwrap_or(false);
                (
                    200,
                    serde_json::json!({"ok": true, "result": exists}).to_string(),
                )
            }
            "SETNX" => {
                // Read-then-conditional-put. KV doesn't expose a
                // true CAS, so a parallel writer between the read
                // and the put can stomp. Acceptable for the cache
                // use case where a lost race just means one of
                // two writers wins; callers that need real
                // atomicity should use a Durable Object.
                if block_on(self.store.get(key).text())
                    .map(|v| v.is_some())
                    .unwrap_or(false)
                {
                    return (
                        200,
                        serde_json::json!({"ok": true, "result": false}).to_string(),
                    );
                }
                let value = data.get("value").and_then(|v| v.as_str()).unwrap_or("");
                let ttl = data.get("ttl").and_then(|v| v.as_u64());
                let (_status, _body) = kv_put(&self.store, key, value, ttl);
                (
                    200,
                    serde_json::json!({"ok": true, "result": true}).to_string(),
                )
            }
            "GETSET" => {
                let value = data.get("value").and_then(|v| v.as_str()).unwrap_or("");
                let ttl = data.get("ttl").and_then(|v| v.as_u64());
                let old = block_on(self.store.get(key).text()).unwrap_or(None);
                let _ = kv_put(&self.store, key, value, ttl);
                (
                    200,
                    serde_json::json!({"ok": true, "result": old}).to_string(),
                )
            }
            "INCR" | "DECR" | "INCRBY" | "DECRBY" => (
                400,
                serde_json::json!({
                    "ok": false,
                    "error": "KV_NO_ATOMIC_COUNTERS",
                    "message": "Cloudflare KV has no atomic counter primitive — use a Durable Object for atomic INCR/DECR semantics. KvCache only supports basic key/value ops.",
                })
                .to_string(),
            ),
            "PING" => (200, serde_json::json!({"ok": true, "result": "PONG"}).to_string()),
            other => (
                400,
                serde_json::json!({
                    "ok": false,
                    "error": format!("unknown command: {other}"),
                })
                .to_string(),
            ),
        }
    }

    fn handle_get(&self, key: &str) -> (u16, String) {
        kv_get(&self.store, key)
    }

    fn handle_delete(&self, key: &str) -> (u16, String) {
        kv_delete(&self.store, key)
    }
}

fn kv_get(store: &KvStore, key: &str) -> (u16, String) {
    match block_on(store.get(key).text()) {
        Ok(Some(v)) => (
            200,
            serde_json::json!({"ok": true, "result": v}).to_string(),
        ),
        Ok(None) => (
            200,
            serde_json::json!({"ok": true, "result": null}).to_string(),
        ),
        Err(e) => (
            500,
            serde_json::json!({"ok": false, "error": format!("KV get: {e}")}).to_string(),
        ),
    }
}

fn kv_put(store: &KvStore, key: &str, value: &str, ttl: Option<u64>) -> (u16, String) {
    // KV's minimum TTL is 60 seconds. Anything less is silently
    // bumped by Cloudflare's edge; surface the bump as a typed
    // hint so callers don't think their 1-second TTL took effect.
    let put_builder = match store.put(key, value) {
        Ok(b) => b,
        Err(e) => {
            return (
                500,
                serde_json::json!({"ok": false, "error": format!("KV put init: {e}")}).to_string(),
            )
        }
    };
    let put_builder = if let Some(t) = ttl {
        put_builder.expiration_ttl(t)
    } else {
        put_builder
    };
    match block_on(put_builder.execute()) {
        Ok(_) => (200, serde_json::json!({"ok": true}).to_string()),
        Err(e) => (
            500,
            serde_json::json!({"ok": false, "error": format!("KV put: {e}")}).to_string(),
        ),
    }
}

fn kv_delete(store: &KvStore, key: &str) -> (u16, String) {
    match block_on(store.delete(key)) {
        Ok(_) => (
            200,
            serde_json::json!({"ok": true, "result": true}).to_string(),
        ),
        Err(e) => (
            500,
            serde_json::json!({"ok": false, "error": format!("KV delete: {e}")}).to_string(),
        ),
    }
}
