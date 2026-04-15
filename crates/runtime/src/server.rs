#[allow(unused_imports)]
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use agentdb_auth::SessionStore;
use agentdb_plugin::PluginRegistry;
use agentdb_policy::PolicyEngine;
use agentdb_sync::{ChangeKind, ChangeLog, SyncCursor};
use tiny_http::{Header, Method, Response, Server};

use crate::rooms::RoomManager;
use crate::sse::SseHub;
use crate::ws::WsHub;
use crate::Runtime;
use crate::metrics::Metrics;
use crate::rate_limit::RateLimiter;
use agentdb_plugin::builtin::cache::CachePlugin;
use agentdb_plugin::builtin::ai_proxy::{AiProxyPlugin, AiMessage};
use crate::pubsub::PubSubBroker;
use crate::cache_handlers::{handle_cache_command, handle_cache_get, handle_cache_delete, handle_pubsub_publish, handle_pubsub_channels, handle_pubsub_history};
use crate::jobs::{JobQueue, JobResult, Priority, Worker};
use crate::scheduler::Scheduler;
use crate::workflows::WorkflowEngine;

// ---------------------------------------------------------------------------
// Streaming body — bridges mpsc::Receiver to std::io::Read for SSE responses
// ---------------------------------------------------------------------------

/// A streaming response body backed by an MPSC channel.
///
/// When used as the body of a `tiny_http::Response`, it causes the server to
/// write data as it arrives through the channel. Dropping the sender closes
/// the stream (EOF).
struct StreamingBody {
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl StreamingBody {
    fn new(rx: std::sync::mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            buf: Vec::new(),
            pos: 0,
        }
    }
}

impl std::io::Read for StreamingBody {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Drain any leftover data from a previous recv that was larger than
        // the caller's buffer.
        if self.pos < self.buf.len() {
            let remaining = &self.buf[self.pos..];
            let n = remaining.len().min(buf.len());
            buf[..n].copy_from_slice(&remaining[..n]);
            self.pos += n;
            if self.pos >= self.buf.len() {
                self.buf.clear();
                self.pos = 0;
            }
            return Ok(n);
        }

        // Block until the next chunk arrives or the sender is dropped.
        match self.rx.recv() {
            Ok(data) if data.is_empty() => Ok(0),
            Ok(data) => {
                let n = data.len().min(buf.len());
                buf[..n].copy_from_slice(&data[..n]);
                if n < data.len() {
                    self.buf = data;
                    self.pos = n;
                }
                Ok(n)
            }
            Err(_) => Ok(0), // Channel closed = EOF
        }
    }
}


/// Global shutdown flag. Set via `request_shutdown()` to trigger graceful exit.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Request a graceful shutdown of the running server.
///
/// This sets the shutdown flag and, if a server handle has been stashed, calls
/// `unblock()` to wake the request loop. Safe to call from any thread or signal
/// handler.
pub fn request_shutdown() {
    SHUTDOWN.store(true, Ordering::SeqCst);
    // If a server handle is available, unblock the request loop so it can
    // observe the flag immediately rather than waiting for the next request.
    if let Some(srv) = SERVER_HANDLE.get() {
        srv.unblock();
    }
}

/// Global handle to the `tiny_http::Server` so `request_shutdown()` can call
/// `unblock()` without requiring callers to hold a reference.
static SERVER_HANDLE: std::sync::OnceLock<Arc<Server>> = std::sync::OnceLock::new();

// ---------------------------------------------------------------------------
// Security headers
// ---------------------------------------------------------------------------

/// Common security headers applied to every response.
fn security_headers() -> Vec<Header> {
    vec![
        Header::from_bytes("X-Content-Type-Options", "nosniff").unwrap(),
        Header::from_bytes("X-Frame-Options", "DENY").unwrap(),
        Header::from_bytes("X-XSS-Protection", "1; mode=block").unwrap(),
    ]
}

/// Add security headers to a response.
fn with_security_headers<R: std::io::Read>(response: Response<R>) -> Response<R> {
    let mut resp = response;
    for header in security_headers() {
        resp = resp.with_header(header);
    }
    resp
}

/// Start the dev server on the given port. Blocks until shutdown.
pub fn start(runtime: Arc<Runtime>, port: u16) -> Result<(), String> {
    start_with_plugins(runtime, port, None)
}

/// Start the dev server with optional plugins. Blocks until shutdown.
pub fn start_with_plugins(
    runtime: Arc<Runtime>,
    port: u16,
    plugins: Option<Arc<PluginRegistry>>,
) -> Result<(), String> {
    let addr = format!("0.0.0.0:{port}");
    let server = Server::http(&addr).map_err(|e| format!("Failed to start server: {e}"))?;
    let server = Arc::new(server);

    // Stash a handle so `request_shutdown()` can unblock the loop.
    let _ = SERVER_HANDLE.set(Arc::clone(&server));

    let session_store = Arc::new(SessionStore::new());
    let magic_codes = Arc::new(agentdb_auth::MagicCodeStore::new());
    let oauth_state = Arc::new(agentdb_auth::OAuthStateStore::new());
    let policy_engine = Arc::new(PolicyEngine::from_manifest(runtime.manifest()));
    let change_log = Arc::new(ChangeLog::new());
    let ws_hub = WsHub::new();
    let sse_hub = SseHub::new();
    let plugin_reg: Arc<PluginRegistry> = plugins.unwrap_or_else(|| Arc::new(PluginRegistry::new(runtime.manifest().clone())));
    let room_mgr = Arc::new(RoomManager::new(120)); // 2 min idle timeout
    let ws_port = port + 1;
    let sse_port = port + 2;

    // Record server start time for the health endpoint.
    let start_time = Instant::now();

    let metrics = Arc::new(Metrics::new());

    // Cache and pub/sub shared instances.
    let cache = Arc::new(CachePlugin::new(100_000));
    let pubsub_broker = Arc::new(PubSubBroker::new(100));

    // Job queue, scheduler, and background workers.
    let job_queue = Arc::new(JobQueue::new(1000));

    // Register built-in framework jobs.
    {
        let cache_ref = Arc::clone(&cache);
        job_queue.register("agentdb.cache.cleanup", Arc::new(move |_job| {
            cache_ref.cleanup_expired();
            JobResult::Success
        }));
        let rooms_ref = Arc::clone(&room_mgr);
        job_queue.register("agentdb.rooms.cleanup", Arc::new(move |_job| {
            rooms_ref.cleanup_idle();
            JobResult::Success
        }));
    }

    let scheduler = Arc::new(Scheduler::new(Arc::clone(&job_queue)));
    // Schedule built-in tasks.
    let _ = scheduler.schedule("agentdb.cache.cleanup", "*/10 * * * *", Arc::new(|_| JobResult::Success));
    let _ = scheduler.schedule("agentdb.rooms.cleanup", "*/5 * * * *", Arc::new(|_| JobResult::Success));

    // Start 2 background workers.
    let _worker_handles: Vec<_> = (0..2)
        .map(|i| {
            let w = Worker::new(Arc::clone(&job_queue), &format!("worker-{i}"));
            w.start()
        })
        .collect();

    // Start the scheduler.
    let _scheduler_handle = Arc::clone(&scheduler).start();

    // Workflow engine: TS runner URL configurable via env, defaults to local Bun server.
    let wf_runner_url = std::env::var("AGENTDB_WORKFLOW_RUNNER_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:9876/run".to_string());
    let workflow_engine = Arc::new(WorkflowEngine::new(&wf_runner_url, 10_000));

    // Rate limiter: 100 requests per IP per 60-second window.
    // Override via AGENTDB_RATE_LIMIT_MAX (count) and AGENTDB_RATE_LIMIT_WINDOW (secs).
    let rl_max: u32 = std::env::var("AGENTDB_RATE_LIMIT_MAX")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    let rl_window: u64 = std::env::var("AGENTDB_RATE_LIMIT_WINDOW")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    let rate_limiter = Arc::new(RateLimiter::new(rl_max, rl_window));

    // CORS origin: configurable via AGENTDB_CORS_ORIGIN env var.
    // Defaults to "*" for dev convenience; should be restricted in production.
    let cors_origin = std::env::var("AGENTDB_CORS_ORIGIN").unwrap_or_else(|_| "*".to_string());

    // Dev mode flag: when false, sensitive data (e.g. magic codes) is omitted from responses.
    let is_dev = std::env::var("AGENTDB_DEV_MODE").map(|v| v == "1" || v == "true").unwrap_or(true);
    // Start WebSocket server on port+1.
    {
        let hub = Arc::clone(&ws_hub);
        std::thread::spawn(move || {
            crate::ws::start_ws_server(hub, ws_port);
        });
    }

    // Start SSE server on port+2.
    {
        let hub = Arc::clone(&sse_hub);
        std::thread::spawn(move || {
            crate::sse::start_sse_server(hub, sse_port);
        });
    }

    eprintln!("agentdb dev server listening on http://localhost:{port}");
    eprintln!("  WebSocket: ws://localhost:{ws_port}");
    eprintln!("  Studio: http://localhost:{port}/studio");
    eprintln!("  API:    http://localhost:{port}/api/entities/<entity>");
    eprintln!("  Auth:   http://localhost:{port}/api/auth/session");
    eprintln!();

    // Use recv() in a loop instead of incoming_requests() so we can share
    // the Arc<Server> with the shutdown path (incoming_requests borrows &self
    // which prevents moving the Arc into another thread).
    loop {
        if SHUTDOWN.load(Ordering::Relaxed) {
            break;
        }

        let mut request = match server.recv() {
            Ok(rq) => rq,
            Err(_) => {
                // recv() returns Err when unblocked or the socket is closed.
                break;
            }
        };

        if SHUTDOWN.load(Ordering::Relaxed) {
            break;
        }

        let rt = Arc::clone(&runtime);
        let ss = Arc::clone(&session_store);
        let pe = Arc::clone(&policy_engine);
        let cl = Arc::clone(&change_log);
        let wh = Arc::clone(&ws_hub);
        let sh = Arc::clone(&sse_hub);
        let mc = Arc::clone(&magic_codes);
        let pr = Arc::clone(&plugin_reg);
        let rm = Arc::clone(&room_mgr);
        let mt = Arc::clone(&metrics);
        let os = Arc::clone(&oauth_state);
        let ca = Arc::clone(&cache);
        let ps = Arc::clone(&pubsub_broker);
        let jq = Arc::clone(&job_queue);
        let sc = Arc::clone(&scheduler);
        let we = Arc::clone(&workflow_engine);
        let cors_origin = cors_origin.clone();
        let is_dev = is_dev;

        let method = request.method().clone();
        let url = request.url().to_string();

        // --- Health check: fast path before auth or body parsing ---
        if url == "/health" && method == Method::Get {
            let uptime = start_time.elapsed().as_secs();
            let body = serde_json::json!({
                "status": "ok",
                "version": "0.1.0",
                "uptime_secs": uptime,
            })
            .to_string();

            let response = with_security_headers(
                Response::from_string(&body)
                    .with_status_code(200u16)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes().to_vec()).unwrap())
            );
            let _ = request.respond(response);
            continue;
        }

        // --- Metrics endpoint: fast path before auth or body parsing ---
        if url == "/metrics" && method == Method::Get {
            let body = mt.snapshot().to_string();
            let response = with_security_headers(
                Response::from_string(&body)
                    .with_status_code(200u16)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes().to_vec()).unwrap())
            );
            let _ = request.respond(response);
            mt.record_request("GET", 200);
            continue;
        }

        // --- Rate limiting: check per-IP request count ---
        let peer_ip = request
            .remote_addr()
            .map(|a| a.ip().to_string())
            .unwrap_or_default();

        if let Err(retry_after) = rate_limiter.check(&peer_ip) {
            let err_body = json_error(
                "RATE_LIMITED",
                &format!("Too many requests. Retry after {retry_after} seconds."),
            );
            let response = with_security_headers(
                Response::from_string(&err_body)
                    .with_status_code(429u16)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes().to_vec()).unwrap())
                    .with_header(Header::from_bytes("Retry-After", retry_after.to_string().as_bytes().to_vec()).unwrap())
            );
            let _ = request.respond(response);
            mt.record_request(method.as_str(), 429);
            continue;
        }

        // Read body before routing (request is consumed by respond).
        let mut body = String::new();
        let _ = request.as_reader().read_to_string(&mut body);

        // --- Max body size check (10 MB) ---
        const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;
        if body.len() > MAX_BODY_SIZE {
            let err_body = json_error(
                "PAYLOAD_TOO_LARGE",
                &format!(
                    "Request body exceeds maximum size of {} bytes",
                    MAX_BODY_SIZE,
                ),
            );
            let response = with_security_headers(
                Response::from_string(&err_body)
                    .with_status_code(413u16)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                    .with_header(Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes().to_vec()).unwrap())
            );
            let _ = request.respond(response);
            mt.record_request(method.as_str(), 413);
            continue;
        }

        // Extract auth token from Authorization header.
        let auth_token: Option<String> = request
            .headers()
            .iter()
            .find(|h| h.field.as_str() == "Authorization" || h.field.as_str() == "authorization")
            .and_then(|h| {
                let val = h.value.as_str();
                val.strip_prefix("Bearer ").map(|t| t.to_string())
            });

        // Resolve auth context from token.
        // Admin token: check AGENTDB_ADMIN_TOKEN env var.
        let admin_token = std::env::var("AGENTDB_ADMIN_TOKEN").ok();
        let auth_ctx = if admin_token.is_some() && auth_token.is_some() && agentdb_auth::constant_time_eq(auth_token.as_deref().unwrap_or("").as_bytes(), admin_token.as_deref().unwrap_or("").as_bytes()) {
            agentdb_auth::AuthContext::admin()
        } else {
            ss.resolve(auth_token.as_deref())
        };

        // --- POST /api/ai/stream — SSE streaming AI completion ---
        if url == "/api/ai/stream" && method == Method::Post {
            let ai_provider = std::env::var("AGENTDB_AI_PROVIDER").unwrap_or_default();
            let ai_key = std::env::var("AGENTDB_AI_API_KEY").unwrap_or_default();
            let ai_model = std::env::var("AGENTDB_AI_MODEL").unwrap_or_default();
            let ai_base = std::env::var("AGENTDB_AI_BASE_URL").unwrap_or_default();

            if ai_key.is_empty() && ai_provider != "custom" {
                let err = json_error("AI_NOT_CONFIGURED", "Set AGENTDB_AI_PROVIDER and AGENTDB_AI_API_KEY");
                let response = with_security_headers(
                    Response::from_string(&err)
                        .with_status_code(503u16)
                        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                        .with_header(Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes().to_vec()).unwrap())
                );
                let _ = request.respond(response);
                mt.record_request("POST", 503);
                continue;
            }

            let parsed: serde_json::Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(_) => {
                    let err = json_error("INVALID_JSON", "Invalid request body");
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(400u16)
                            .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                            .with_header(Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes().to_vec()).unwrap())
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 400);
                    continue;
                }
            };

            let messages: Vec<AiMessage> = match parsed.get("messages").and_then(|m| m.as_array()) {
                Some(arr) => arr.iter().filter_map(|m| {
                    let role = m.get("role")?.as_str()?.to_string();
                    let content = m.get("content")?.as_str()?.to_string();
                    Some(AiMessage { role, content })
                }).collect(),
                None => {
                    let err = json_error("MISSING_FIELD", "\"messages\" array is required");
                    let response = with_security_headers(
                        Response::from_string(&err)
                            .with_status_code(400u16)
                            .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
                            .with_header(Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes().to_vec()).unwrap())
                    );
                    let _ = request.respond(response);
                    mt.record_request("POST", 400);
                    continue;
                }
            };

            // Override model from request body if provided.
            let model = parsed.get("model")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
                .unwrap_or(ai_model);

            let proxy = match ai_provider.as_str() {
                "anthropic" => AiProxyPlugin::anthropic(&ai_key, &model),
                "openai" => AiProxyPlugin::openai(&ai_key, &model),
                "custom" => AiProxyPlugin::custom_with_model(&ai_base, &ai_key, &model),
                _ => AiProxyPlugin::openai(&ai_key, &model),
            };

            // Set up a channel-based streaming body so tiny_http streams
            // data to the client as chunks arrive from the AI provider.
            let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
            let streaming_body = StreamingBody::new(rx);

            // Spawn the provider request on a background thread. Each chunk
            // is formatted as an SSE event and pushed through the channel.
            std::thread::spawn(move || {
                let result = proxy.stream_completion(&messages, &mut |chunk| {
                    let sse = format!("data: {}

", serde_json::json!({
                        "choices": [{"index": 0, "delta": {"content": chunk}}]
                    }));
                    let _ = tx.send(sse.into_bytes());
                });

                // Send a final event indicating completion or error.
                match result {
                    Ok(_) => {
                        let _ = tx.send(b"data: [DONE]

".to_vec());
                    }
                    Err(e) => {
                        let err_event = format!("data: {}

", serde_json::json!({"error": {"message": e, "type": "stream_error"}}));
                        let _ = tx.send(err_event.into_bytes());
                    }
                }
                // tx is dropped here, which causes StreamingBody::read to return 0 (EOF).
            });

            let response = with_security_headers(
                Response::new(
                    tiny_http::StatusCode(200),
                    vec![
                        Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                        Header::from_bytes("Cache-Control", "no-cache").unwrap(),
                        Header::from_bytes("Connection", "keep-alive").unwrap(),
                        Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes().to_vec()).unwrap(),
                    ],
                    streaming_body,
                    None, // unknown content length = chunked transfer
                    None,
                )
            );
            let _ = request.respond(response);
            mt.record_request("POST", 200);
            continue;
        }

        // Studio route (returns HTML, not JSON).
        let (status, response_body, content_type, is_studio) = if (url == "/studio" || url == "/studio/") && method == Method::Get {
            let base = format!("http://localhost:{port}");
            let html = agentdb_studio_api::generate_studio_html(rt.manifest(), &base);
            (200u16, html, "text/html", true)
        } else {
            // Run plugin middleware.
            if let Err(e) = pr.run_on_request(method.as_str(), &url, &auth_ctx) {
                (e.status, json_error(&e.code, &e.message), "application/json", false)
            } else if let Some((s, b)) = pr.try_handle_route(method.as_str(), &url, &body, &auth_ctx) {
                // Plugin handled the route.
                (s, b, "application/json", false)
            } else {
                let (s, b) = route(&rt, &ss, &mc, &pe, &cl, &wh, &sh, &rm, &os, &ca, &ps, &jq, &sc, &we, &auth_ctx, &method, &url, &body, auth_token.as_deref(), is_dev);
                (s, b, "application/json", false)
            }
        };

        let mut response = Response::from_string(&response_body)
            .with_status_code(status)
            .with_header(Header::from_bytes("Content-Type", content_type).unwrap())
            .with_header(Header::from_bytes("Access-Control-Allow-Origin", cors_origin.as_bytes().to_vec()).unwrap())
            .with_header(
                Header::from_bytes(
                    "Access-Control-Allow-Methods",
                    "GET, POST, PATCH, DELETE, OPTIONS",
                )
                .unwrap(),
            )
            .with_header(Header::from_bytes("Access-Control-Allow-Headers", "Content-Type, Authorization").unwrap());

        // Add Content-Security-Policy for Studio HTML responses.
        if is_studio {
            response = response.with_header(
                Header::from_bytes(
                    "Content-Security-Policy",
                    "default-src 'self' 'unsafe-inline' 'unsafe-eval' https://cdn.tailwindcss.com https://unpkg.com",
                ).unwrap(),
            );
        }

        let response = with_security_headers(response);

        let _ = request.respond(response);
        mt.record_request(method.as_str(), status);
    }

    eprintln!("Shutting down gracefully...");
    Ok(())
}

fn route(
    rt: &Runtime,
    ss: &SessionStore,
    mc: &agentdb_auth::MagicCodeStore,
    pe: &PolicyEngine,
    cl: &ChangeLog,
    wh: &WsHub,
    sh: &SseHub,
    rm: &RoomManager,
    oauth_state: &agentdb_auth::OAuthStateStore,
    cache: &CachePlugin,
    pubsub: &PubSubBroker,
    job_queue: &JobQueue,
    scheduler: &Scheduler,
    workflows: &WorkflowEngine,
    auth_ctx: &agentdb_auth::AuthContext,
    method: &Method,
    url: &str,
    body: &str,
    auth_token: Option<&str>,
    is_dev: bool,
) -> (u16, String) {
    // CORS preflight
    if method.as_str() == "OPTIONS" {
        return (204, String::new());
    }

    // GET /api/manifest
    if url == "/api/manifest" && *method == Method::Get {
        return (
            200,
            serde_json::to_string(rt.manifest()).unwrap_or_else(|_| "{}".into()),
        );
    }


    // GET /api/openapi.json — OpenAPI 3.0.3 specification
    if url == "/api/openapi.json" && *method == Method::Get {
        let spec = crate::openapi::generate_openapi(rt.manifest(), "");
        return (
            200,
            serde_json::to_string(&spec).unwrap_or_else(|_| "{}".into()),
        );
    }
    // POST /api/auth/session — create a session
    if url == "/api/auth/session" && *method == Method::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let user_id = match data.get("user_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return (400, json_error("MISSING_USER_ID", "user_id is required")),
        };
        let session = ss.create(user_id);
        return (
            201,
            serde_json::json!({"token": session.token, "user_id": session.user_id}).to_string(),
        );
    }

    // GET /api/auth/me — get current auth context
    if url == "/api/auth/me" && *method == Method::Get {
        let ctx = ss.resolve(auth_token);
        return (200, serde_json::to_string(&ctx).unwrap_or_else(|_| "{}".into()));
    }

    // POST /api/auth/guest — create a guest session
    if url == "/api/auth/guest" && *method == Method::Post {
        let session = ss.create_guest();
        return (
            201,
            serde_json::json!({"token": session.token, "user_id": session.user_id, "guest": true}).to_string(),
        );
    }

    // POST /api/auth/upgrade — upgrade a guest session to a real user
    if url == "/api/auth/upgrade" && *method == Method::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let user_id = match data.get("user_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return (400, json_error("MISSING_USER_ID", "user_id is required")),
        };
        if let Some(token) = auth_token {
            if ss.upgrade(token, user_id.clone()) {
                return (200, serde_json::json!({"upgraded": true, "user_id": user_id}).to_string());
            }
        }
        return (400, json_error("UPGRADE_FAILED", "No valid session to upgrade"));
    }

    // POST /api/auth/magic/send — send a magic code to an email
    if url == "/api/auth/magic/send" && *method == Method::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let email = match data.get("email").and_then(|v| v.as_str()) {
            Some(e) => e.to_string(),
            None => return (400, json_error("MISSING_EMAIL", "email is required")),
        };
        let code = mc.create(&email);
        // In production, this would send an email via a configured transport.
        // In dev mode, we include the code in the response for testing convenience.
        if is_dev {
            return (
                200,
                serde_json::json!({"sent": true, "email": email, "dev_code": code}).to_string(),
            );
        } else {
            return (
                200,
                serde_json::json!({"sent": true, "email": email}).to_string(),
            );
        }
    }

    // POST /api/auth/magic/verify — verify a magic code and create a session
    if url == "/api/auth/magic/verify" && *method == Method::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let email = match data.get("email").and_then(|v| v.as_str()) {
            Some(e) => e,
            None => return (400, json_error("MISSING_EMAIL", "email is required")),
        };
        let code = match data.get("code").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return (400, json_error("MISSING_CODE", "code is required")),
        };
        if mc.verify(email, code) {
            // Auto-create or find user in the User entity.
            let user_id = match rt.lookup("User", "email", email) {
                Ok(Some(row)) => row["id"].as_str().unwrap_or("").to_string(),
                _ => {
                    // Create a new user.
                    let now = format!("{}Z", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
                    rt.insert("User", &serde_json::json!({"email": email, "displayName": email, "createdAt": now}))
                        .unwrap_or_else(|_| email.to_string())
                }
            };
            let session = ss.create(user_id.clone());
            return (
                200,
                serde_json::json!({"token": session.token, "user_id": user_id}).to_string(),
            );
        }
        return (401, json_error("INVALID_CODE", "Invalid or expired code"));
    }

    // GET /api/auth/providers — list available OAuth providers
    if url == "/api/auth/providers" && *method == Method::Get {
        let registry = agentdb_auth::OAuthRegistry::from_env();
        let providers: Vec<serde_json::Value> = ["google", "github"]
            .iter()
            .filter_map(|p| {
                registry.get(p).map(|c| {
                    serde_json::json!({
                        "provider": p,
                        "auth_url": c.auth_url(),
                    })
                })
            })
            .collect();
        return (200, serde_json::to_string(&providers).unwrap_or_else(|_| "[]".into()));
    }

    // GET /api/auth/login/:provider — redirect to OAuth provider
    if let Some(provider) = url.strip_prefix("/api/auth/login/") {
        let provider = provider.split('?').next().unwrap_or(provider);
        if *method == Method::Get {
            let registry = agentdb_auth::OAuthRegistry::from_env();
            if let Some(config) = registry.get(provider) {
                let state = oauth_state.create(provider);
                return (
                    200,
                    serde_json::json!({"redirect": config.auth_url_with_state(&state), "state": state}).to_string(),
                );
            }
            return (404, json_error_with_hint(
                "PROVIDER_NOT_FOUND",
                &format!("OAuth provider \"{provider}\" is not configured"),
                "Set AGENTDB_OAUTH_GOOGLE_CLIENT_ID / AGENTDB_OAUTH_GITHUB_CLIENT_ID environment variables"
            ));
        }
    }

    // POST /api/auth/callback/:provider — handle OAuth callback with code
    if let Some(provider) = url.strip_prefix("/api/auth/callback/") {
        let provider = provider.split('?').next().unwrap_or(provider);
        if *method == Method::Post {
            let data: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
            };
            let _code = data.get("code").and_then(|v| v.as_str()).unwrap_or("");
            let email = data.get("email").and_then(|v| v.as_str());
            let name = data.get("name").and_then(|v| v.as_str());

            // Validate the OAuth state parameter to prevent CSRF attacks.
            let state = data.get("state").and_then(|v| v.as_str());
            match state {
                Some(s) if oauth_state.validate(s, provider) => { /* state is valid, proceed */ }
                _ => return (403, json_error("OAUTH_INVALID_STATE", "Invalid or missing OAuth state parameter")),
            }

            // In a real implementation, we would also exchange the code for tokens
            // with the provider. For now, if the client provides email (from the
            // OAuth provider's response), we create/find the user and issue a session.
            if let Some(email) = email {
                let user_id = match rt.lookup("User", "email", email) {
                    Ok(Some(row)) => row["id"].as_str().unwrap_or("").to_string(),
                    _ => {
                        let display_name = name.unwrap_or(email);
                        let now = format!("{}Z", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
                        rt.insert("User", &serde_json::json!({"email": email, "displayName": display_name, "createdAt": now}))
                            .unwrap_or_else(|_| email.to_string())
                    }
                };
                let session = ss.create(user_id.clone());
                return (
                    200,
                    serde_json::json!({
                        "token": session.token,
                        "user_id": user_id,
                        "provider": provider,
                    }).to_string(),
                );
            }
            return (400, json_error("MISSING_EMAIL", "OAuth callback requires email from provider"));
        }
    }

    // DELETE /api/auth/session — revoke current session
    if url == "/api/auth/session" && *method == Method::Delete {
        if let Some(token) = auth_token {
            ss.revoke(token);
        }
        return (200, serde_json::json!({"revoked": true}).to_string());
    }

    // GET /api/sync/pull — pull changes since cursor
    if url.starts_with("/api/sync/pull") && *method == Method::Get {
        let since: u64 = url
            .split("since=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let resp = cl.pull(&SyncCursor { last_seq: since }, 100);
        return (
            200,
            serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
        );
    }

    // POST /api/sync/push — apply client mutations
    if url == "/api/sync/push" && *method == Method::Post {
        let push_req: agentdb_sync::PushRequest = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };

        let mut applied = 0u32;
        let mut errors: Vec<String> = Vec::new();

        for change in &push_req.changes {
            match change.kind {
                ChangeKind::Insert => {
                    if let Some(ref data) = change.data {
                        match rt.insert(&change.entity, data) {
                            Ok(id) => {
                                cl.append(&change.entity, &id, ChangeKind::Insert, change.data.clone());
                                applied += 1;
                            }
                            Err(e) => errors.push(format!("insert {}: {}", change.entity, e.message)),
                        }
                    }
                }
                ChangeKind::Update => {
                    if let Some(ref data) = change.data {
                        match rt.update(&change.entity, &change.row_id, data) {
                            Ok(_) => {
                                cl.append(&change.entity, &change.row_id, ChangeKind::Update, change.data.clone());
                                applied += 1;
                            }
                            Err(e) => errors.push(format!("update {}/{}: {}", change.entity, change.row_id, e.message)),
                        }
                    }
                }
                ChangeKind::Delete => {
                    match rt.delete(&change.entity, &change.row_id) {
                        Ok(_) => {
                            cl.append(&change.entity, &change.row_id, ChangeKind::Delete, None);
                            applied += 1;
                        }
                        Err(e) => errors.push(format!("delete {}/{}: {}", change.entity, change.row_id, e.message)),
                    }
                }
            }
        }

        return (
            200,
            serde_json::json!({
                "applied": applied,
                "errors": errors,
                "cursor": {"last_seq": cl.len()}
            }).to_string(),
        );
    }

    // -----------------------------------------------------------------------
    // Rooms API
    // -----------------------------------------------------------------------

    // POST /api/rooms/join — join a room
    if url == "/api/rooms/join" && *method == Method::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let room = match data.get("room").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return (400, json_error("MISSING_ROOM", "room is required")),
        };
        let user_id = data.get("user_id").and_then(|v| v.as_str())
            .or_else(|| auth_ctx.user_id.as_deref());
        let user_id = match user_id {
            Some(u) => u,
            None => return (401, json_error("AUTH_REQUIRED", "user_id or auth token required")),
        };
        let user_data = data.get("data").cloned();

        let (snapshot, join_event) = match rm.join(room, user_id, user_data) {
            Ok(result) => result,
            Err(e) => return (429, json_error(&e.code, &e.message)),
        };

        // Broadcast the join event to WS/SSE.
        if let Ok(json) = serde_json::to_string(&join_event) {
            wh.broadcast_presence(&json);
            sh.broadcast_message(&json);
        }

        return (200, serde_json::json!({
            "joined": room,
            "snapshot": snapshot,
        }).to_string());
    }

    // POST /api/rooms/leave — leave a room
    if url == "/api/rooms/leave" && *method == Method::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let room = match data.get("room").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return (400, json_error("MISSING_ROOM", "room is required")),
        };
        let user_id = data.get("user_id").and_then(|v| v.as_str())
            .or_else(|| auth_ctx.user_id.as_deref());
        let user_id = match user_id {
            Some(u) => u,
            None => return (401, json_error("AUTH_REQUIRED", "user_id or auth token required")),
        };

        if let Some(leave_event) = rm.leave(room, user_id) {
            if let Ok(json) = serde_json::to_string(&leave_event) {
                wh.broadcast_presence(&json);
                sh.broadcast_message(&json);
            }
            return (200, serde_json::json!({"left": room}).to_string());
        }
        return (404, json_error("NOT_IN_ROOM", "User is not in this room"));
    }

    // POST /api/rooms/presence — update presence in a room
    if url == "/api/rooms/presence" && *method == Method::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let room = match data.get("room").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return (400, json_error("MISSING_ROOM", "room is required")),
        };
        let user_id = data.get("user_id").and_then(|v| v.as_str())
            .or_else(|| auth_ctx.user_id.as_deref());
        let user_id = match user_id {
            Some(u) => u,
            None => return (401, json_error("AUTH_REQUIRED", "user_id or auth token required")),
        };
        let presence_data = data.get("data").cloned().unwrap_or(serde_json::json!({}));

        if let Some(presence_event) = rm.set_presence(room, user_id, presence_data) {
            if let Ok(json) = serde_json::to_string(&presence_event) {
                wh.broadcast_presence(&json);
                sh.broadcast_message(&json);
            }
            return (200, serde_json::json!({"updated": true}).to_string());
        }
        return (404, json_error("NOT_IN_ROOM", "User is not in this room"));
    }

    // POST /api/rooms/broadcast — broadcast a message to a room
    if url == "/api/rooms/broadcast" && *method == Method::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let room = match data.get("room").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return (400, json_error("MISSING_ROOM", "room is required")),
        };
        let topic = match data.get("topic").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return (400, json_error("MISSING_TOPIC", "topic is required")),
        };
        let sender = data.get("user_id").and_then(|v| v.as_str())
            .or_else(|| auth_ctx.user_id.as_deref());
        let broadcast_data = data.get("data").cloned().unwrap_or(serde_json::json!({}));

        if let Some(broadcast_event) = rm.broadcast(room, sender, topic, broadcast_data) {
            if let Ok(json) = serde_json::to_string(&broadcast_event) {
                wh.broadcast_presence(&json);
                sh.broadcast_message(&json);
            }
            return (200, serde_json::json!({"broadcasted": true}).to_string());
        }
        return (404, json_error("ROOM_NOT_FOUND", "Room does not exist"));
    }

    // GET /api/rooms — list all active rooms
    if url == "/api/rooms" && *method == Method::Get {
        let room_names = rm.list_rooms();
        let rooms: Vec<serde_json::Value> = room_names
            .iter()
            .map(|name| serde_json::json!({
                "name": name,
                "members": rm.room_size(name),
            }))
            .collect();
        return (200, serde_json::to_string(&rooms).unwrap_or_else(|_| "[]".into()));
    }

    // GET /api/rooms/:room — get room members
    if let Some(room_name) = url.strip_prefix("/api/rooms/") {
        let room_name = room_name.split('?').next().unwrap_or(room_name);
        // Skip sub-paths that are handled above.
        if *method == Method::Get
            && room_name != "join"
            && room_name != "leave"
            && room_name != "presence"
            && room_name != "broadcast"
        {
            let members = rm.members(room_name);
            return (200, serde_json::json!({
                "room": room_name,
                "members": members,
                "count": members.len(),
            }).to_string());
        }
    }

    // POST /api/link — link two entities
    if url == "/api/link" && *method == Method::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let entity = data.get("entity").and_then(|v| v.as_str()).unwrap_or("");
        let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let relation = data.get("relation").and_then(|v| v.as_str()).unwrap_or("");
        let target_id = data.get("target_id").and_then(|v| v.as_str()).unwrap_or("");

        match rt.link(entity, id, relation, target_id) {
            Ok(true) => return (200, serde_json::json!({"linked": true}).to_string()),
            Ok(false) => return (404, json_error("NOT_FOUND", "Source entity not found")),
            Err(e) => return (400, json_error(&e.code, &e.message)),
        }
    }

    // POST /api/unlink — unlink two entities
    if url == "/api/unlink" && *method == Method::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let entity = data.get("entity").and_then(|v| v.as_str()).unwrap_or("");
        let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let relation = data.get("relation").and_then(|v| v.as_str()).unwrap_or("");

        match rt.unlink(entity, id, relation) {
            Ok(true) => return (200, serde_json::json!({"unlinked": true}).to_string()),
            Ok(false) => return (404, json_error("NOT_FOUND", "Source entity not found")),
            Err(e) => return (400, json_error(&e.code, &e.message)),
        }
    }

    // POST /api/files/upload — upload a file (stores in local uploads/ dir)
    if url == "/api/files/upload" && *method == Method::Post {
        let uploads_dir = std::path::Path::new("uploads");
        let _ = std::fs::create_dir_all(uploads_dir);

        let file_id = format!("file_{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos());
        let file_path = uploads_dir.join(&file_id);
        match std::fs::write(&file_path, body.as_bytes()) {
            Ok(()) => {
                return (
                    201,
                    serde_json::json!({
                        "id": file_id,
                        "url": format!("/api/files/{}", file_id),
                        "size": body.len(),
                    }).to_string(),
                );
            }
            Err(e) => {
                return (500, json_error_safe("FILE_WRITE_FAILED", "Failed to store file", &format!("Failed to store file: {e}")));
            }
        }
    }

    // GET /api/files/:id — serve a file
    if let Some(file_id) = url.strip_prefix("/api/files/") {
        let file_id = file_id.split('?').next().unwrap_or(file_id);
        if !is_valid_file_id(file_id) {
            return (400, json_error("INVALID_FILE_ID", "Invalid file ID"));
        }
        if *method == Method::Get {
            let file_path = std::path::Path::new("uploads").join(file_id);
            match std::fs::read_to_string(&file_path) {
                Ok(content) => return (200, content),
                Err(_) => return (404, json_error("FILE_NOT_FOUND", "File not found")),
            }
        }
    }

    // POST /api/transact — atomic multi-entity transaction
    if url == "/api/transact" && *method == Method::Post {
        let ops: Vec<serde_json::Value> = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };

        let mut results: Vec<serde_json::Value> = Vec::new();
        let mut all_ok = true;

        // Use _with_conn variants to avoid deadlock (single lock for entire transaction).
        {
            let conn = rt.lock_conn_pub();
            if let Ok(conn) = conn {
                let _ = conn.execute("BEGIN", []);
                let mut rollback = false;

                for op in &ops {
                    let op_type = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
                    let entity = op.get("entity").and_then(|v| v.as_str()).unwrap_or("");

                    match op_type {
                        "insert" => {
                            let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                            match rt.insert_with_conn(&conn, entity, &data) {
                                Ok(id) => {
                                    let seq = cl.append(entity, &id, ChangeKind::Insert, Some(data.clone()));
                                    broadcast_change(wh, sh, seq, entity, &id, "insert", Some(&data));
                                    results.push(serde_json::json!({"op": "insert", "id": id}));
                                }
                                Err(e) => {
                                    rollback = true;
                                    results.push(serde_json::json!({"op": "insert", "error": e.message}));
                                }
                            }
                        }
                        "update" => {
                            let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                            match rt.update_with_conn(&conn, entity, id, &data) {
                                Ok(_) => {
                                    let seq = cl.append(entity, id, ChangeKind::Update, Some(data.clone()));
                                    broadcast_change(wh, sh, seq, entity, id, "update", Some(&data));
                                    results.push(serde_json::json!({"op": "update", "id": id}));
                                }
                                Err(e) => {
                                    rollback = true;
                                    results.push(serde_json::json!({"op": "update", "error": e.message}));
                                }
                            }
                        }
                        "delete" => {
                            let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            match rt.delete_with_conn(&conn, entity, id) {
                                Ok(_) => {
                                    let seq = cl.append(entity, id, ChangeKind::Delete, None);
                                    broadcast_change(wh, sh, seq, entity, id, "delete", None);
                                    results.push(serde_json::json!({"op": "delete", "id": id}));
                                }
                                Err(e) => {
                                    rollback = true;
                                    results.push(serde_json::json!({"op": "delete", "error": e.message}));
                                }
                            }
                        }
                        _ => {
                            results.push(serde_json::json!({"op": op_type, "error": "unknown operation"}));
                        }
                    }
                }

                if rollback {
                    let _ = conn.execute("ROLLBACK", []);
                    all_ok = false;
                } else {
                    let _ = conn.execute("COMMIT", []);
                }
            }
        }

        return (
            if all_ok { 200 } else { 400 },
            serde_json::json!({
                "committed": all_ok,
                "results": results,
            }).to_string(),
        );
    }

    // POST /api/query/filtered — query with operators ($not, $gt, $in, etc.)
    if url.starts_with("/api/query/") && *method == Method::Post {
        let entity = url.strip_prefix("/api/query/").unwrap_or("").split('?').next().unwrap_or("");
        if !entity.is_empty() && entity != "filtered" {
            let filter: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
            };
            match rt.query_filtered(entity, &filter) {
                Ok(rows) => return (200, serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())),
                Err(e) => return (400, json_error(&e.code, &e.message)),
            }
        }
    }

    // GET /api/lookup/:entity/:field/:value — lookup by unique field
    if let Some(path) = url.strip_prefix("/api/lookup/") {
        let path = path.split('?').next().unwrap_or(path);
        let parts: Vec<&str> = path.splitn(3, '/').collect();
        if parts.len() == 3 && *method == Method::Get {
            match rt.lookup(parts[0], parts[1], parts[2]) {
                Ok(Some(row)) => return (200, serde_json::to_string(&row).unwrap_or_else(|_| "{}".into())),
                Ok(None) => return (404, json_error("NOT_FOUND", &format!("{}.{} = {} not found", parts[0], parts[1], parts[2]))),
                Err(e) => return (400, json_error(&e.code, &e.message)),
            }
        }
    }

    // POST /api/query — execute a graph query
    if url == "/api/query" && *method == Method::Post {
        let query: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        match rt.query_graph(&query) {
            Ok(result) => return (200, serde_json::to_string(&result).unwrap_or_else(|_| "{}".into())),
            Err(e) => return (400, json_error(&e.code, &e.message)),
        }
    }

    // POST /api/actions/:name — execute a named action
    if let Some(action_name) = url.strip_prefix("/api/actions/") {
        let action_name = action_name.split('?').next().unwrap_or(action_name);
        if *method != Method::Post {
            return (405, json_error("METHOD_NOT_ALLOWED", "Actions require POST"));
        }

        let input: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };

        // Check action policy.
        let policy_check = pe.check_action(action_name, auth_ctx, Some(&input));
        if !policy_check.is_allowed() {
            if let agentdb_policy::PolicyResult::Denied { reason, .. } = policy_check {
                return (403, json_error("POLICY_DENIED", &reason));
            }
        }

        // Find the action in the manifest.
        let manifest = rt.manifest();
        let action_def = manifest.actions.iter().find(|a| a.name == action_name);
        if action_def.is_none() {
            let available: Vec<&str> = manifest.actions.iter().map(|a| a.name.as_str()).collect();
            return (404, json_error_with_hint(
                "ACTION_NOT_FOUND",
                &format!("Unknown action: \"{action_name}\""),
                &format!("Available actions: [{}]", available.join(", "))
            ));
        }
        let action_def = action_def.unwrap();

        // Validate required input fields.
        let input_obj = input.as_object();
        for field in &action_def.input {
            if !field.optional {
                let has_field = input_obj
                    .and_then(|o| o.get(&field.name))
                    .map(|v| !v.is_null())
                    .unwrap_or(false);
                if !has_field {
                    let required: Vec<String> = action_def.input.iter()
                        .filter(|f| !f.optional)
                        .map(|f| format!("{}: {}", f.name, f.field_type))
                        .collect();
                    return (
                        400,
                        json_error_with_hint(
                            "ACTION_MISSING_INPUT",
                            &format!("Required input field \"{}\" (type: {}) is missing for action \"{}\"", field.name, field.field_type, action_name),
                            &format!("Required fields: [{}]", required.join(", "))
                        ),
                    );
                }
            }
        }

        // Execute: determine which entity to mutate based on input fields.
        // Convention: if action input has an id field referencing an entity, it's an update/toggle.
        // If not, it's an insert into the entity targeted by non-id fields.
        // For now, actions return the input validation result + a success marker.
        // The app defines what the action does — we validate and pass through.
        return (
            200,
            serde_json::json!({
                "action": action_name,
                "input": input,
                "executed": true,
            })
            .to_string(),
        );
    }

    // GET /api/export — export ALL data from ALL entities (admin only)
    if url == "/api/export" && *method == Method::Get {
        if !auth_ctx.is_admin {
            return (403, json_error("FORBIDDEN", "Admin access required for data export"));
        }
        let manifest = rt.manifest();
        let mut entities_map = serde_json::Map::new();
        let mut counts_map = serde_json::Map::new();
        for ent in &manifest.entities {
            match rt.list(&ent.name) {
                Ok(rows) => {
                    counts_map.insert(ent.name.clone(), serde_json::json!(rows.len()));
                    entities_map.insert(ent.name.clone(), serde_json::json!(rows));
                }
                Err(e) => {
                    return (500, json_error_safe("EXPORT_FAILED", "Export operation failed", &format!("Failed to export {}: {}", ent.name, e.message)));
                }
            }
        }
        let now = chrono_now_iso();
        return (200, serde_json::json!({
            "exported_at": now,
            "entities": entities_map,
            "counts": counts_map,
        }).to_string());
    }

    // GET /api/export/<entity> — export a single entity (admin only)
    if let Some(entity_name) = url.strip_prefix("/api/export/") {
        let entity_name = entity_name.split('?').next().unwrap_or(entity_name);
        if *method == Method::Get && !entity_name.is_empty() {
            if !auth_ctx.is_admin {
                return (403, json_error("FORBIDDEN", "Admin access required for data export"));
            }
            match rt.list(entity_name) {
                Ok(rows) => {
                    let now = chrono_now_iso();
                    let mut entities_map = serde_json::Map::new();
                    let mut counts_map = serde_json::Map::new();
                    counts_map.insert(entity_name.to_string(), serde_json::json!(rows.len()));
                    entities_map.insert(entity_name.to_string(), serde_json::json!(rows));
                    return (200, serde_json::json!({
                        "exported_at": now,
                        "entities": entities_map,
                        "counts": counts_map,
                    }).to_string());
                }
                Err(e) => return (400, json_error(&e.code, &e.message)),
            }
        }
    }


    // GET /api/entities/<entity>/cursor — cursor-based pagination
    if let Some(rest) = url.strip_prefix("/api/entities/") {
        let rest_no_qs = rest.split('?').next().unwrap_or(rest);
        if let Some(entity_name) = rest_no_qs.strip_suffix("/cursor") {
            if *method == Method::Get {
                let after: Option<&str> = url
                    .split("after=")
                    .nth(1)
                    .and_then(|s| s.split('&').next())
                    .filter(|s| !s.is_empty());
                let limit: usize = url
                    .split("limit=")
                    .nth(1)
                    .and_then(|s| s.split('&').next())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(20)
                    .min(100);

                return match rt.list_after(entity_name, after, limit + 1) {
                    Ok(rows) => {
                        let has_more = rows.len() > limit;
                        let page: Vec<serde_json::Value> = rows.into_iter().take(limit).collect();
                        let next_cursor = page.last().and_then(|r| r.get("id")).and_then(|v| v.as_str()).map(|s| s.to_string());
                        (200, serde_json::json!({
                            "data": page,
                            "next_cursor": next_cursor,
                            "has_more": has_more,
                        }).to_string())
                    }
                    Err(e) => (400, json_error(&e.code, &e.message)),
                };
            }
        }
    }

    // POST /api/batch — independent batch operations (no transaction wrapping)
    if url == "/api/batch" && *method == Method::Post {
        let batch: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let ops = match batch.get("operations").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => return (400, json_error("MISSING_OPERATIONS", "Request body must contain an \"operations\" array")),
        };

        let mut results: Vec<serde_json::Value> = Vec::new();
        let mut succeeded: u32 = 0;
        let mut failed: u32 = 0;

        for op in ops {
            let op_type = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let entity = op.get("entity").and_then(|v| v.as_str()).unwrap_or("");

            match op_type {
                "insert" => {
                    let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                    match rt.insert(entity, &data) {
                        Ok(id) => {
                            let seq = cl.append(entity, &id, ChangeKind::Insert, Some(data.clone()));
                            broadcast_change(wh, sh, seq, entity, &id, "insert", Some(&data));
                            results.push(serde_json::json!({"op": "insert", "id": id, "ok": true}));
                            succeeded += 1;
                        }
                        Err(e) => {
                            results.push(serde_json::json!({"op": "insert", "ok": false, "error": e.message}));
                            failed += 1;
                        }
                    }
                }
                "update" => {
                    let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                    match rt.update(entity, id, &data) {
                        Ok(updated) => {
                            if updated {
                                let seq = cl.append(entity, id, ChangeKind::Update, Some(data.clone()));
                                broadcast_change(wh, sh, seq, entity, id, "update", Some(&data));
                            }
                            results.push(serde_json::json!({"op": "update", "id": id, "ok": true}));
                            succeeded += 1;
                        }
                        Err(e) => {
                            results.push(serde_json::json!({"op": "update", "id": id, "ok": false, "error": e.message}));
                            failed += 1;
                        }
                    }
                }
                "delete" => {
                    let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    match rt.delete(entity, id) {
                        Ok(deleted) => {
                            if deleted {
                                let seq = cl.append(entity, id, ChangeKind::Delete, None);
                                broadcast_change(wh, sh, seq, entity, id, "delete", None);
                            }
                            results.push(serde_json::json!({"op": "delete", "id": id, "ok": true}));
                            succeeded += 1;
                        }
                        Err(e) => {
                            results.push(serde_json::json!({"op": "delete", "id": id, "ok": false, "error": e.message}));
                            failed += 1;
                        }
                    }
                }
                _ => {
                    results.push(serde_json::json!({"op": op_type, "ok": false, "error": "unknown operation"}));
                    failed += 1;
                }
            }
        }

        return (200, serde_json::json!({
            "results": results,
            "succeeded": succeeded,
            "failed": failed,
        }).to_string());
    }

    // Parse /api/entities/<entity>[/<id>]
    if let Some(path) = url.strip_prefix("/api/entities/") {
        let path = path.split('?').next().unwrap_or(path);
        let segments: Vec<&str> = path.splitn(2, '/').collect();
        let entity_name = segments[0];
        let entity_id = segments.get(1).filter(|s| !s.is_empty()).copied();

        // Check entity-level read policies for GET requests.
        if *method == Method::Get {
            let check = pe.check_entity_read(entity_name, auth_ctx, None);
            if !check.is_allowed() {
                if let agentdb_policy::PolicyResult::Denied { reason, .. } = check {
                    return (403, json_error_with_hint("POLICY_DENIED", &reason, "Check your auth token or the policy rules in your schema"));
                }
            }
        }

        return match (method, entity_id) {
            (m, None) if *m == Method::Get => handle_list(rt, entity_name, url),
            (m, None) if *m == Method::Post => handle_insert(rt, cl, wh, sh, entity_name, body),
            (m, Some(id)) if *m == Method::Get => handle_get(rt, entity_name, id),
            (m, Some(id)) if *m == Method::Patch => handle_update(rt, cl, wh, sh, entity_name, id, body),
            (m, Some(id)) if *m == Method::Delete => handle_delete(rt, cl, wh, sh, entity_name, id),
            _ => (405, json_error("METHOD_NOT_ALLOWED", "Method not allowed")),
        };
    }



    // -----------------------------------------------------------------------
    // Cache API (delegated to shared cache_handlers module)
    // -----------------------------------------------------------------------

    // POST /api/cache -- execute a cache command
    if url == "/api/cache" && *method == Method::Post {
        return handle_cache_command(cache, body);
    }

    // GET /api/cache/:key -- quick get shorthand
    if let Some(cache_key) = url.strip_prefix("/api/cache/") {
        let cache_key = cache_key.split('?').next().unwrap_or(cache_key);
        if *method == Method::Get && !cache_key.is_empty() {
            return handle_cache_get(cache, cache_key);
        }
        if *method == Method::Delete && !cache_key.is_empty() {
            return handle_cache_delete(cache, cache_key);
        }
    }

    // -----------------------------------------------------------------------
    // Pub/Sub API (delegated to shared cache_handlers module)
    // -----------------------------------------------------------------------

    // POST /api/pubsub/publish
    if url == "/api/pubsub/publish" && *method == Method::Post {
        return handle_pubsub_publish(pubsub, body);
    }

    // GET /api/pubsub/channels
    if url == "/api/pubsub/channels" && *method == Method::Get {
        return handle_pubsub_channels(pubsub);
    }

    // GET /api/pubsub/history/:channel
    if let Some(channel_name) = url.strip_prefix("/api/pubsub/history/") {
        let channel_name = channel_name.split('?').next().unwrap_or(channel_name);
        if *method == Method::Get && !channel_name.is_empty() {
            return handle_pubsub_history(pubsub, channel_name, url);
        }
    }

    // -----------------------------------------------------------------------
    // Jobs API
    // -----------------------------------------------------------------------

    // POST /api/jobs -- enqueue a job
    if url == "/api/jobs" && *method == Method::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let name = match data.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return (400, json_error("MISSING_NAME", "name is required")),
        };
        let payload = data.get("payload").cloned().unwrap_or(serde_json::json!({}));
        let priority = data.get("priority")
            .and_then(|v| v.as_str())
            .map(Priority::from_str_loose)
            .unwrap_or(Priority::Normal);
        let delay = data.get("delay_secs").and_then(|v| v.as_u64()).unwrap_or(0);
        let max_retries = data.get("max_retries").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
        let queue = data.get("queue").and_then(|v| v.as_str()).unwrap_or("default");

        let id = job_queue.enqueue_with_options(name, payload, priority, delay, max_retries, queue);
        return (201, serde_json::json!({"id": id, "status": "pending"}).to_string());
    }

    // GET /api/jobs/stats -- queue statistics
    if url == "/api/jobs/stats" && *method == Method::Get {
        let stats = job_queue.stats();
        return (200, serde_json::to_string(&stats).unwrap_or_else(|_| "{}".into()));
    }

    // GET /api/jobs/dead -- dead letter queue
    if url == "/api/jobs/dead" && *method == Method::Get {
        let dead = job_queue.dead_letters();
        return (200, serde_json::to_string(&dead).unwrap_or_else(|_| "[]".into()));
    }

    // POST /api/jobs/dead/:id/retry -- retry a dead letter
    if let Some(rest) = url.strip_prefix("/api/jobs/dead/") {
        let rest = rest.split('?').next().unwrap_or(rest);
        if let Some(job_id) = rest.strip_suffix("/retry") {
            if *method == Method::Post && !job_id.is_empty() {
                if job_queue.retry_dead(job_id) {
                    return (200, serde_json::json!({"retried": true, "id": job_id}).to_string());
                }
                return (404, json_error("NOT_FOUND", "Job not found in dead letter queue"));
            }
        }
    }

    // GET /api/jobs -- list recent jobs
    if url.starts_with("/api/jobs") && *method == Method::Get {
        let path = url.split('?').next().unwrap_or(url);
        if path == "/api/jobs" {
            let status_filter = url.split("status=").nth(1).and_then(|s| s.split('&').next());
            let queue_filter = url.split("queue=").nth(1).and_then(|s| s.split('&').next());
            let limit: usize = url.split("limit=").nth(1)
                .and_then(|s| s.split('&').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(50)
                .min(200);
            let jobs = job_queue.list_jobs(status_filter, queue_filter, limit);
            return (200, serde_json::to_string(&jobs).unwrap_or_else(|_| "[]".into()));
        }
    }

    // GET /api/jobs/:id -- get job status
    if let Some(job_id) = url.strip_prefix("/api/jobs/") {
        let job_id = job_id.split('?').next().unwrap_or(job_id);
        if *method == Method::Get
            && !job_id.is_empty()
            && job_id != "stats"
            && job_id != "dead"
        {
            if let Some(job) = job_queue.get_job(job_id) {
                return (200, serde_json::to_string(&job).unwrap_or_else(|_| "{}".into()));
            }
            return (404, json_error("NOT_FOUND", &format!("Job {job_id} not found")));
        }
    }

    // -----------------------------------------------------------------------
    // Scheduler API
    // -----------------------------------------------------------------------

    // GET /api/scheduler -- list scheduled tasks
    if url == "/api/scheduler" && *method == Method::Get {
        let tasks = scheduler.list_tasks();
        return (200, serde_json::to_string(&tasks).unwrap_or_else(|_| "[]".into()));
    }

    // POST /api/scheduler/trigger/:name -- manually trigger a scheduled task
    if let Some(task_name) = url.strip_prefix("/api/scheduler/trigger/") {
        let task_name = task_name.split('?').next().unwrap_or(task_name);
        if *method == Method::Post && !task_name.is_empty() {
            if scheduler.trigger(task_name) {
                return (200, serde_json::json!({"triggered": true, "task": task_name}).to_string());
            }
            return (404, json_error("NOT_FOUND", &format!("Scheduled task \"{}\" not found", task_name)));
        }
    }

    // -----------------------------------------------------------------------
    // Workflow Engine API
    // -----------------------------------------------------------------------

    // GET /api/workflows/definitions -- list registered workflow definitions
    if url == "/api/workflows/definitions" && *method == Method::Get {
        let defs = workflows.definitions();
        return (200, serde_json::to_string(&defs).unwrap_or_else(|_| "[]".into()));
    }

    // POST /api/workflows/start -- start a new workflow instance
    if url == "/api/workflows/start" && *method == Method::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };
        let name = match data.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_string(),
            None => return (400, json_error("MISSING_FIELD", "\"name\" is required")),
        };
        let input = data.get("input").cloned().unwrap_or(serde_json::json!({}));
        match workflows.start(&name, input) {
            Ok(id) => return (201, serde_json::json!({"id": id}).to_string()),
            Err(e) => return (400, json_error("WORKFLOW_START_FAILED", &e)),
        }
    }

    // GET /api/workflows -- list workflow instances (optional ?status=running)
    if url.starts_with("/api/workflows") && !url.starts_with("/api/workflows/") && *method == Method::Get {
        let status_filter = url
            .split("status=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .and_then(|s| match s {
                "pending" => Some(crate::workflows::WorkflowStatus::Pending),
                "running" => Some(crate::workflows::WorkflowStatus::Running),
                "sleeping" => Some(crate::workflows::WorkflowStatus::Sleeping),
                "waiting" => Some(crate::workflows::WorkflowStatus::WaitingForEvent),
                "completed" => Some(crate::workflows::WorkflowStatus::Completed),
                "failed" => Some(crate::workflows::WorkflowStatus::Failed),
                "cancelled" => Some(crate::workflows::WorkflowStatus::Cancelled),
                _ => None,
            });
        let instances = workflows.list(status_filter.as_ref());
        return (200, serde_json::to_string(&instances).unwrap_or_else(|_| "[]".into()));
    }

    // Routes with workflow ID: /api/workflows/:id/...
    if let Some(rest) = url.strip_prefix("/api/workflows/") {
        let rest = rest.split('?').next().unwrap_or(rest);
        // Split into id and sub-path.
        let (wf_id, sub) = match rest.find('/') {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };

        if !wf_id.is_empty() && !wf_id.starts_with("definitions") {
            match (method, sub) {
                // GET /api/workflows/:id -- get workflow instance
                (m, None) if *m == Method::Get => {
                    return match workflows.get(wf_id) {
                        Some(inst) => (200, serde_json::to_string(&inst).unwrap_or_else(|_| "{}".into())),
                        None => (404, json_error("NOT_FOUND", &format!("Workflow {wf_id} not found"))),
                    };
                }
                // POST /api/workflows/:id/advance -- advance to next step
                (m, Some("advance")) if *m == Method::Post => {
                    return match workflows.advance(wf_id) {
                        Ok(status) => (200, serde_json::json!({"status": status}).to_string()),
                        Err(e) => (400, json_error("WORKFLOW_ADVANCE_FAILED", &e)),
                    };
                }
                // POST /api/workflows/:id/event -- send an event
                (m, Some("event")) if *m == Method::Post => {
                    let data: serde_json::Value = match serde_json::from_str(body) {
                        Ok(v) => v,
                        Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
                    };
                    let event = match data.get("event").and_then(|v| v.as_str()) {
                        Some(e) => e.to_string(),
                        None => return (400, json_error("MISSING_FIELD", "\"event\" is required")),
                    };
                    let event_data = data.get("data").cloned().unwrap_or(serde_json::json!({}));
                    return match workflows.send_event(wf_id, &event, event_data) {
                        Ok(()) => (200, serde_json::json!({"ok": true}).to_string()),
                        Err(e) => (400, json_error("WORKFLOW_EVENT_FAILED", &e)),
                    };
                }
                // POST /api/workflows/:id/cancel -- cancel workflow
                (m, Some("cancel")) if *m == Method::Post => {
                    return match workflows.cancel(wf_id) {
                        Ok(()) => (200, serde_json::json!({"cancelled": true}).to_string()),
                        Err(e) => (400, json_error("WORKFLOW_CANCEL_FAILED", &e)),
                    };
                }
                _ => {}
            }
        }
    }

    // POST /api/ai/complete — non-streaming AI completion
    if url == "/api/ai/complete" && *method == Method::Post {
        let ai_provider = std::env::var("AGENTDB_AI_PROVIDER").unwrap_or_default();
        let ai_key = std::env::var("AGENTDB_AI_API_KEY").unwrap_or_default();
        let ai_model = std::env::var("AGENTDB_AI_MODEL").unwrap_or_default();
        let ai_base = std::env::var("AGENTDB_AI_BASE_URL").unwrap_or_default();

        if ai_key.is_empty() && ai_provider != "custom" {
            return (503, json_error("AI_NOT_CONFIGURED", "Set AGENTDB_AI_PROVIDER and AGENTDB_AI_API_KEY"));
        }

        let parsed: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
        };

        let messages: Vec<AiMessage> = match parsed.get("messages").and_then(|m| m.as_array()) {
            Some(arr) => arr.iter().filter_map(|m| {
                let role = m.get("role")?.as_str()?.to_string();
                let content = m.get("content")?.as_str()?.to_string();
                Some(AiMessage { role, content })
            }).collect(),
            None => return (400, json_error("MISSING_FIELD", "\"messages\" array is required")),
        };

        if messages.is_empty() {
            return (400, json_error("EMPTY_MESSAGES", "At least one message is required"));
        }

        let model = parsed.get("model")
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
            .unwrap_or(ai_model);

        let proxy = match ai_provider.as_str() {
            "anthropic" => AiProxyPlugin::anthropic(&ai_key, &model),
            "openai" => AiProxyPlugin::openai(&ai_key, &model),
            "custom" => AiProxyPlugin::custom_with_model(&ai_base, &ai_key, &model),
            _ => AiProxyPlugin::openai(&ai_key, &model),
        };

        return match proxy.completion(&messages) {
            Ok(response_text) => {
                (200, serde_json::json!({
                    "response": response_text,
                    "model": model,
                }).to_string())
            }
            Err(e) => (502, json_error("AI_REQUEST_FAILED", &e)),
        };
    }



    (404, json_error_with_hint(
        "NOT_FOUND",
        &format!("No API route matches {url}"),
        "Available endpoints: /api/entities/<entity>, /api/actions/<name>, /api/query, /api/auth/*, /api/sync/*, /api/files/*, /api/cache, /api/pubsub/*, /api/jobs, /api/scheduler, /api/workflows, /api/ai/*, /studio"
    ))
}

fn handle_list(rt: &Runtime, entity: &str, url: &str) -> (u16, String) {
    // Parse ?limit=N&offset=N from URL.
    let limit: Option<usize> = url.split("limit=").nth(1).and_then(|s| s.split('&').next()).and_then(|s| s.parse().ok());
    let offset: usize = url.split("offset=").nth(1).and_then(|s| s.split('&').next()).and_then(|s| s.parse().ok()).unwrap_or(0);

    match rt.list(entity) {
        Ok(rows) => {
            let total = rows.len();
            let paginated: Vec<serde_json::Value> = if let Some(lim) = limit {
                rows.into_iter().skip(offset).take(lim).collect()
            } else {
                rows.into_iter().skip(offset).collect()
            };
            (200, serde_json::json!({
                "data": paginated,
                "total": total,
                "offset": offset,
                "limit": limit,
            }).to_string())
        }
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

fn handle_get(rt: &Runtime, entity: &str, id: &str) -> (u16, String) {
    match rt.get_by_id(entity, id) {
        Ok(Some(row)) => (200, serde_json::to_string(&row).unwrap_or_else(|_| "{}".into())),
        Ok(None) => (404, json_error("NOT_FOUND", &format!("{entity} with id \"{id}\" not found"))),
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

fn handle_insert(rt: &Runtime, cl: &ChangeLog, wh: &WsHub, sh: &SseHub, entity: &str, body: &str) -> (u16, String) {
    let data: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
    };
    match rt.insert(entity, &data) {
        Ok(id) => {
            let seq = cl.append(entity, &id, ChangeKind::Insert, Some(data.clone()));
            broadcast_change(wh, sh, seq, entity, &id, "insert", Some(&data));
            (201, serde_json::json!({"id": id}).to_string())
        }
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

fn handle_update(rt: &Runtime, cl: &ChangeLog, wh: &WsHub, sh: &SseHub, entity: &str, id: &str, body: &str) -> (u16, String) {
    let data: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return (400, json_error_safe("INVALID_JSON", "Invalid request body", &format!("Invalid JSON: {e}"))),
    };
    match rt.update(entity, id, &data) {
        Ok(true) => {
            let seq = cl.append(entity, id, ChangeKind::Update, Some(data.clone()));
            broadcast_change(wh, sh, seq, entity, id, "update", Some(&data));
            (200, serde_json::json!({"updated": true}).to_string())
        }
        Ok(false) => (404, json_error("NOT_FOUND", &format!("{entity}/{id} not found"))),
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

fn handle_delete(rt: &Runtime, cl: &ChangeLog, wh: &WsHub, sh: &SseHub, entity: &str, id: &str) -> (u16, String) {
    match rt.delete(entity, id) {
        Ok(true) => {
            let seq = cl.append(entity, id, ChangeKind::Delete, None);
            broadcast_change(wh, sh, seq, entity, id, "delete", None);
            (200, serde_json::json!({"deleted": true}).to_string())
        }
        Ok(false) => (404, json_error("NOT_FOUND", &format!("{entity}/{id} not found"))),
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

fn broadcast_change(
    wh: &WsHub,
    sh: &SseHub,
    seq: u64,
    entity: &str,
    row_id: &str,
    kind: &str,
    data: Option<&serde_json::Value>,
) {
    let event = agentdb_sync::ChangeEvent {
        seq,
        entity: entity.to_string(),
        row_id: row_id.to_string(),
        kind: match kind {
            "insert" => agentdb_sync::ChangeKind::Insert,
            "update" => agentdb_sync::ChangeKind::Update,
            "delete" => agentdb_sync::ChangeKind::Delete,
            _ => return,
        },
        data: data.cloned(),
        timestamp: String::new(),
    };
    wh.broadcast(&event);
    sh.broadcast(&event);
}

fn json_error(code: &str, message: &str) -> String {
    serde_json::json!({"error": {"code": code, "message": message}}).to_string()
}

fn json_error_with_hint(code: &str, message: &str, hint: &str) -> String {
    serde_json::json!({"error": {"code": code, "message": message, "hint": hint}}).to_string()
}

/// Log the internal error details to stderr but return a safe, generic message to the client.
/// Use this for errors that could leak implementation details (SQL errors, parse errors, etc.).
fn json_error_safe(code: &str, user_message: &str, internal: &str) -> String {
    eprintln!("[error] {code}: {internal}");
    json_error(code, user_message)
}

/// Returns the current UTC time as an ISO 8601 string.
///
/// Uses `std::time::SystemTime` to avoid pulling in the `chrono` crate.
fn chrono_now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Convert epoch seconds to a rough ISO 8601 representation.
    // For a dev-server export timestamp, second-level precision is sufficient.
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Calculate year/month/day from days since epoch (1970-01-01).
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = is_leap(y);
    let month_days: [i64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md {
            m = i;
            break;
        }
        remaining -= md;
    }
    let d = remaining + 1;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m + 1,
        d,
        hours,
        minutes,
        seconds
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Validate that a file ID does not contain path traversal sequences.
/// Defense-in-depth: even though IDs are server-generated, the GET endpoint
/// accepts arbitrary user input.
fn is_valid_file_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains("..")
        && !id.contains('/')
        && !id.contains('\\')
        && !id.starts_with('.')
}
