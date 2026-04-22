//! Standalone cache server.
//!
//! Runs a lightweight HTTP server that exposes only cache and pub/sub
//! endpoints. This allows the cache to be deployed independently of the
//! main statecraft server for horizontal scaling.
//!
//! # Usage
//!
//! ```text
//! statecraft cache --port 6380 --max-keys 100000 --max-history 100
//! ```
//!
//! # Endpoints
//!
//! - `POST /cache`              -- execute a cache command (same protocol as `/api/cache`)
//! - `GET  /cache/:key`         -- shorthand GET
//! - `DELETE /cache/:key`       -- shorthand DELETE
//! - `POST /pubsub/publish`     -- publish a message
//! - `GET  /pubsub/channels`    -- list channels
//! - `GET  /pubsub/history/:ch` -- channel history
//! - `GET  /health`             -- health check

use std::sync::Arc;

use statecraft_plugin::builtin::cache::CachePlugin;
use tiny_http::{Header, Method, Response, Server};

use crate::cache_handlers::{
    handle_cache_command, handle_cache_delete, handle_cache_get, handle_pubsub_channels,
    handle_pubsub_history, handle_pubsub_publish,
};
use crate::pubsub::PubSubBroker;

/// Start a standalone cache server on the given port.
///
/// This blocks the calling thread, serving requests in a synchronous loop.
/// It runs independently of the main statecraft server -- no auth, no entities,
/// no sync. Just the cache and pub/sub.
pub fn start_cache_server(port: u16, max_keys: usize, max_history: usize) -> Result<(), String> {
    start_cache_server_with_options(port, max_keys, max_history, None, false)
}

/// Start a standalone cache server with optional RESP protocol support.
///
/// When `resp_port` is `Some(port)`, a RESP-compatible TCP server is also
/// started on that port, allowing `redis-cli` and any Redis client library
/// to connect directly.
///
/// When `resp_only` is `true`, only the RESP server is started (no HTTP).
pub fn start_cache_server_with_options(
    port: u16,
    max_keys: usize,
    max_history: usize,
    resp_port: Option<u16>,
    resp_only: bool,
) -> Result<(), String> {
    let cache = Arc::new(CachePlugin::new(max_keys));
    let pubsub = Arc::new(PubSubBroker::new(max_history));

    // Start the RESP server on a background thread if requested.
    if let Some(rp) = resp_port {
        let cache_for_resp = Arc::clone(&cache);
        std::thread::spawn(move || {
            crate::resp_server::start_resp_server(cache_for_resp, rp);
        });
    }

    // If resp-only mode, block on the RESP server instead of HTTP.
    if resp_only {
        let rp = resp_port.unwrap_or(6379);
        tracing::warn!("[cache] RESP-only mode -- no HTTP server started");
        // The RESP server was already spawned above if resp_port was Some.
        // If it was None (user said --resp-only without --resp-port), start
        // it on the default port on this thread.
        if resp_port.is_none() {
            crate::resp_server::start_resp_server(cache, rp);
        } else {
            // Block forever so the process doesn't exit. The RESP server
            // thread is doing the real work.
            loop {
                std::thread::park();
            }
        }
        return Ok(());
    }

    // Start the HTTP server.
    let addr = format!("0.0.0.0:{port}");
    let server =
        Server::http(&addr).map_err(|e| format!("Failed to start cache server: {e}"))?;

    tracing::warn!("statecraft cache server listening on http://localhost:{port}");
    tracing::warn!("  Cache:  POST http://localhost:{port}/cache");
    tracing::warn!("  PubSub: POST http://localhost:{port}/pubsub/publish");
    tracing::warn!("  Health: GET  http://localhost:{port}/health");

    for mut request in server.incoming_requests() {
        let cache = Arc::clone(&cache);
        let pubsub = Arc::clone(&pubsub);

        let mut body = String::new();
        let _ = std::io::Read::read_to_string(request.as_reader(), &mut body);

        let method = request.method().clone();
        let url = request.url().to_string();

        let (status, response_body) = route_request(&cache, &pubsub, &method, &url, &body);

        let response = Response::from_string(&response_body)
            .with_status_code(status)
            .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
            .with_header(Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap());

        let _ = request.respond(response);
    }

    Ok(())
}

/// Route a request to the appropriate handler.
fn route_request(
    cache: &CachePlugin,
    pubsub: &PubSubBroker,
    method: &Method,
    url: &str,
    body: &str,
) -> (u16, String) {
    // CORS preflight
    if method.as_str() == "OPTIONS" {
        return (204, String::new());
    }

    // Health check
    if url == "/health" && *method == Method::Get {
        let info = cache.info();
        return (
            200,
            serde_json::json!({
                "status": "ok",
                "mode": "standalone",
                "keys": cache.dbsize(),
                "stats": info,
            })
            .to_string(),
        );
    }

    // POST /cache -- execute a cache command
    if url == "/cache" && *method == Method::Post {
        return handle_cache_command(cache, body);
    }

    // GET or DELETE /cache/:key
    if let Some(key) = url.strip_prefix("/cache/") {
        let key = key.split('?').next().unwrap_or(key);
        if !key.is_empty() {
            if *method == Method::Get {
                return handle_cache_get(cache, key);
            }
            if *method == Method::Delete {
                return handle_cache_delete(cache, key);
            }
        }
    }

    // POST /pubsub/publish
    if url == "/pubsub/publish" && *method == Method::Post {
        return handle_pubsub_publish(pubsub, body);
    }

    // GET /pubsub/channels
    if url == "/pubsub/channels" && *method == Method::Get {
        return handle_pubsub_channels(pubsub);
    }

    // GET /pubsub/history/:channel
    if let Some(channel) = url.strip_prefix("/pubsub/history/") {
        let channel = channel.split('?').next().unwrap_or(channel);
        if *method == Method::Get && !channel.is_empty() {
            return handle_pubsub_history(pubsub, channel, url);
        }
    }

    (
        404,
        serde_json::json!({
            "error": {
                "code": "NOT_FOUND",
                "message": "Not found"
            }
        })
        .to_string(),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (CachePlugin, PubSubBroker) {
        (CachePlugin::new(1000), PubSubBroker::new(100))
    }

    #[test]
    fn health_check() {
        let (cache, pubsub) = setup();
        let (status, body) = route_request(&cache, &pubsub, &Method::Get, "/health", "");
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["mode"], "standalone");
    }

    #[test]
    fn cache_command_via_route() {
        let (cache, pubsub) = setup();
        let (status, _) = route_request(
            &cache,
            &pubsub,
            &Method::Post,
            "/cache",
            r#"{"cmd": "SET", "key": "x", "value": "1"}"#,
        );
        assert_eq!(status, 200);

        let (status, body) = route_request(
            &cache,
            &pubsub,
            &Method::Get,
            "/cache/x",
            "",
        );
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["result"], "1");
    }

    #[test]
    fn delete_via_route() {
        let (cache, pubsub) = setup();
        cache.set("del_me", "val", None);
        let (status, _) = route_request(
            &cache,
            &pubsub,
            &Method::Delete,
            "/cache/del_me",
            "",
        );
        assert_eq!(status, 200);
        assert!(cache.get("del_me").is_none());
    }

    #[test]
    fn pubsub_publish_via_route() {
        let (cache, pubsub) = setup();
        let (status, body) = route_request(
            &cache,
            &pubsub,
            &Method::Post,
            "/pubsub/publish",
            r#"{"channel": "test", "message": "hi"}"#,
        );
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["ok"], true);
    }

    #[test]
    fn pubsub_channels_via_route() {
        let (cache, pubsub) = setup();
        let (status, body) = route_request(
            &cache,
            &pubsub,
            &Method::Get,
            "/pubsub/channels",
            "",
        );
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["ok"], true);
    }

    #[test]
    fn pubsub_history_via_route() {
        let (cache, pubsub) = setup();
        pubsub.publish("events", "e1");
        let (status, body) = route_request(
            &cache,
            &pubsub,
            &Method::Get,
            "/pubsub/history/events?limit=10",
            "",
        );
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["result"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn not_found() {
        let (cache, pubsub) = setup();
        let (status, _) = route_request(
            &cache,
            &pubsub,
            &Method::Get,
            "/nonexistent",
            "",
        );
        assert_eq!(status, 404);
    }

    #[test]
    fn cors_preflight() {
        let (cache, pubsub) = setup();
        let (status, body) = route_request(
            &cache,
            &pubsub,
            &Method::Options,
            "/cache",
            "",
        );
        assert_eq!(status, 204);
        assert!(body.is_empty());
    }
}
