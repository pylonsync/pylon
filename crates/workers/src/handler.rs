//! Cloudflare Workers fetch handler.
//!
//! Compiled only when the `workers` feature is enabled, since it depends on
//! the `worker` crate (which requires `wasm32-unknown-unknown`).
//!
//! To build the Workers bundle:
//! ```sh
//! cargo install worker-build
//! worker-build --release --features workers
//! ```

use pylon_http::HttpMethod;
use pylon_router::{route, RouterContext};
// Explicit imports — NOT `use worker::*` because that would
// shadow `std::result::Result<T, E>` with `worker::Result<T>`
// (single type parameter), breaking every two-parameter Result
// signature in the file with "type alias takes 1 generic argument
// but 2 were supplied".
use worker::{
    event, Context, D1Database, D1Type, Env, Fetch, Headers, MessageBatch, Method, Request,
    RequestInit, Response, Result as WResult,
};

use crate::d1_store::{D1DataStore, D1Executor};
use crate::noop_adapters::NoopAll;

// ---------------------------------------------------------------------------
// D1 executor backed by the real Workers D1 binding
// ---------------------------------------------------------------------------

pub struct WorkerD1Executor {
    db: D1Database,
}

impl WorkerD1Executor {
    pub fn new(db: D1Database) -> Self {
        Self { db }
    }
}

impl D1Executor for WorkerD1Executor {
    fn execute(&self, sql: &str, params: &[serde_json::Value]) -> std::result::Result<u64, String> {
        let stmt = self.db.prepare(sql);
        // worker 0.5's bind_refs takes &[&D1Argument]. Build a Vec
        // of owned D1Type values from the JSON args, then bind a
        // borrowed view.
        let typed = json_params_to_d1(params);
        let bound = stmt.bind_refs(typed.iter()).map_err(|e| e.to_string())?;
        let result = futures::executor::block_on(bound.run()).map_err(|e| e.to_string())?;
        Ok(result
            .meta()
            .ok()
            .flatten()
            .and_then(|m| m.changes)
            .unwrap_or(0) as u64)
    }

    fn query(
        &self,
        sql: &str,
        params: &[serde_json::Value],
    ) -> std::result::Result<Vec<serde_json::Value>, String> {
        let stmt = self.db.prepare(sql);
        let typed = json_params_to_d1(params);
        let bound = stmt.bind_refs(typed.iter()).map_err(|e| e.to_string())?;
        let result = futures::executor::block_on(bound.all()).map_err(|e| e.to_string())?;
        let rows = result
            .results::<serde_json::Value>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }
}

/// Map JSON values to worker 0.5's `D1Type` enum. Mirrors the SQL
/// type coercion the SQLite/PG backends do for JSON inputs.
fn json_params_to_d1(params: &[serde_json::Value]) -> Vec<D1Type<'static>> {
    params
        .iter()
        .map(|v| match v {
            serde_json::Value::Null => D1Type::Null,
            serde_json::Value::Bool(b) => D1Type::Integer(if *b { 1 } else { 0 }),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    // Saturate to i32 — D1 integers map to JS
                    // numbers; values beyond i32 round-trip
                    // through f64 with precision loss anyway.
                    D1Type::Integer(i.try_into().unwrap_or(i32::MAX))
                } else if let Some(f) = n.as_f64() {
                    D1Type::Real(f)
                } else {
                    D1Type::Null
                }
            }
            // Plain strings bind as their raw value. NOT
            // serde_json::to_string(s) — that would quote-wrap
            // every column and corrupt D1 lookups against
            // existing plain-string rows.
            serde_json::Value::String(s) => D1Type::Text(Box::leak(s.clone().into_boxed_str())),
            // Arrays + objects serialize as their JSON form so
            // the column holds parseable JSON text. Caller is
            // responsible for parsing on read.
            other => D1Type::Text(Box::leak(
                serde_json::to_string(other)
                    .unwrap_or_default()
                    .into_boxed_str(),
            )),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Fetch handler
// ---------------------------------------------------------------------------

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: Context) -> WResult<Response> {
    let method = HttpMethod::from_str(&req.method().to_string());
    let url = req.path();
    let body = req.text().await.unwrap_or_default();

    let auth_token = req
        .headers()
        .get("Authorization")?
        .and_then(|v| v.strip_prefix("Bearer ").map(String::from));

    // Snapshot every request header into the (String, String) pair
    // form RouterContext expects. Header names lower-cased per the
    // router's contract so webhook handlers can find e.g.
    // "stripe-signature" without case-folding at every call site.
    let request_headers: Vec<(String, String)> = req
        .headers()
        .entries()
        .map(|(name, value)| (name.to_ascii_lowercase(), value))
        .collect();
    // Cloudflare's client IP lives in `cf-connecting-ip`. Honored
    // by the framework's rate limiter + audit logs without needing
    // PYLON_TRUST_PROXY_HOPS (Workers proxy stack is implicit).
    let peer_ip = request_headers
        .iter()
        .find(|(k, _)| k == "cf-connecting-ip")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();

    // Load manifest from a KV/env binding.
    let manifest_json = env
        .var("PYLON_MANIFEST_JSON")
        .map(|v| v.to_string())
        .unwrap_or_else(|_| "{}".into());
    let manifest: pylon_kernel::AppManifest =
        serde_json::from_str(&manifest_json).unwrap_or_else(|_| empty_manifest());

    let d1 = env.d1("PYLON_DB")?;
    let executor = WorkerD1Executor::new(d1);
    let store = D1DataStore::new(executor, manifest.clone());

    let session_store = pylon_auth::SessionStore::new();
    let magic_codes = pylon_auth::MagicCodeStore::new();
    let oauth_state = pylon_auth::OAuthStateStore::new();
    let policy_engine = pylon_policy::PolicyEngine::from_manifest(&manifest);
    let change_log = pylon_sync::ChangeLog::new();
    let auth_ctx = session_store.resolve(auth_token.as_deref());
    let noop = NoopAll::new(&manifest);
    // EmailSender: HTTP-POST transport. When PYLON_EMAIL_WEBHOOK_URL
    // is set in the env, ctx.email.send POSTs a `{to, subject, body}`
    // JSON envelope to that URL. The operator points it at any
    // HTTP endpoint that delivers mail — Resend, Postmark, SES via
    // API Gateway, or another Cloudflare Worker. Without the env,
    // sends log a warning + drop silently so apps don't 500 on
    // first deploy.
    let email_url = env
        .var("PYLON_EMAIL_WEBHOOK_URL")
        .ok()
        .map(|v| v.to_string())
        .filter(|s| !s.is_empty());
    let email_auth = env
        .var("PYLON_EMAIL_WEBHOOK_BEARER")
        .ok()
        .map(|v| v.to_string())
        .filter(|s| !s.is_empty());
    let email = HttpWebhookEmailSender {
        url: email_url,
        bearer: email_auth,
    };

    // Optional real bindings — if the operator declared them in
    // wrangler.toml, plug in the live adapters; otherwise fall
    // back to NoopAll's typed-503 stubs so missing bindings
    // surface as <FOO>_BINDING_REQUIRED rather than mysteriously
    // failing.
    let kv_cache_opt = env.kv("PYLON_CACHE").ok().map(crate::KvCache::new);
    let r2_files_opt = env.bucket("PYLON_FILES").ok().map(crate::R2Files::new);
    let rooms_opt = if env.durable_object("PYLON_ROOMS").is_ok() {
        Some(crate::WorkersRooms::new(env.clone(), "PYLON_ROOMS"))
    } else {
        None
    };
    let pubsub_opt = if env.durable_object("PYLON_PUBSUB").is_ok() {
        Some(crate::WorkersPubSub::new(env.clone(), "PYLON_PUBSUB"))
    } else {
        None
    };
    let jobs_opt = crate::WorkersJobs::from_env(&env);
    let cache_ref: &dyn pylon_router::CacheOps = match kv_cache_opt.as_ref() {
        Some(c) => c,
        None => &noop,
    };
    let files_ref: &dyn pylon_router::FileOps = match r2_files_opt.as_ref() {
        Some(f) => f,
        None => &noop,
    };
    let rooms_ref: &dyn pylon_router::RoomOps = match rooms_opt.as_ref() {
        Some(r) => r,
        None => &noop,
    };
    let pubsub_ref: &dyn pylon_router::PubSubOps = match pubsub_opt.as_ref() {
        Some(p) => p,
        None => &noop,
    };
    let jobs_ref: &dyn pylon_router::JobOps = match jobs_opt.as_ref() {
        Some(j) => j,
        None => &noop,
    };
    let cookie_config = pylon_auth::CookieConfig::from_env(
        &pylon_auth::CookieConfig::default_name_for(&manifest.name),
    );

    // RouterContext on wasm32 omits the auth-flow fields that
    // depend on native crypto (api_keys, siwe, phone_codes,
    // passkeys, org_sso, saml). Those modules don't compile to
    // wasm32 (ring/k256/ureq/samael chain), so the Workers target
    // simply doesn't serve OAuth/SAML/SCIM/phone/passkey endpoints.
    // Customers needing those features deploy on Fly (full runtime)
    // until either we ship WebCrypto-backed wasm replacements or
    // Workers gains those bindings natively.
    let ctx = RouterContext {
        store: &store,
        session_store: &session_store,
        magic_codes: &magic_codes,
        oauth_state: &oauth_state,
        // Native-only stores not initialized on wasm32; matching
        // cfg-gates on the field declarations in router/src/lib.rs.
        // Stores that ARE pure (orgs, audit, verification, etc.)
        // get fresh in-memory instances per request — no persistence
        // here, which is the documented limit of the bare Workers
        // target without a separate state store wired in.
        orgs: &pylon_auth::org::OrgStore::new(
            std::sync::Arc::new(EmptyDataStore),
            manifest.auth.org.clone(),
        ),
        verification: &pylon_auth::verification::VerificationStore::new(),
        audit: &pylon_auth::audit::AuditStore::new(),
        trusted_devices: &pylon_auth::trusted_device::InMemoryTrustedDeviceStore::new(),
        account_store: &pylon_auth::AccountStore::new(),
        policy_engine: &policy_engine,
        change_log: &change_log,
        notifier: &pylon_router::NoopNotifier,
        rooms: rooms_ref,
        cache: cache_ref,
        pubsub: pubsub_ref,
        jobs: jobs_ref,
        scheduler: &noop,
        workflows: &noop,
        files: files_ref,
        openapi: &noop,
        functions: None,
        email: &email,
        shards: None,
        plugin_hooks: &pylon_router::NoopPluginHooks,
        auth_ctx: &auth_ctx,
        is_dev: false,
        request_headers: &request_headers,
        peer_ip: &peer_ip,
        cookie_config: &cookie_config,
        response_headers: std::cell::RefCell::new(Vec::new()),
        trusted_origins: &[],
    };

    let (status, response_body, _ct) = route(&ctx, method, &url, &body, auth_token.as_deref());

    let mut headers = Headers::new();
    headers.set("Content-Type", "application/json")?;
    headers.set("Access-Control-Allow-Origin", "*")?;
    headers.set(
        "Access-Control-Allow-Methods",
        "GET, POST, PATCH, DELETE, OPTIONS",
    )?;
    headers.set(
        "Access-Control-Allow-Headers",
        "Content-Type, Authorization",
    )?;

    Ok(Response::ok(response_body)?
        .with_status(status)
        .with_headers(headers))
}

// ---------------------------------------------------------------------------
// Queue consumer — drains MessageBatch and updates the registry
// ---------------------------------------------------------------------------

/// Consumer arm for the `PYLON_JOBS` Cloudflare Queue. Wired
/// via `[[queues.consumers]]` in wrangler.toml. For each
/// message:
///
/// 1. Decode the envelope (the same JSON we wrote in
///    `WorkersJobs::enqueue`).
/// 2. Update the registry record `jobs:<id>` to status="running"
///    so the dashboard sees the transition immediately.
/// 3. Dispatch to the function runner — TODO: today this is a
///    placeholder that marks completed without actually running
///    the user's handler, because the Workers fetch handler's
///    FnOpsImpl isn't wired into the queue path. The next iteration
///    routes the message into the running fetch-side function
///    runner by POSTing internally to /api/jobs/run with the
///    envelope.
/// 4. On success → status="completed". On error → retry via the
///    Queues retry mechanism + bump attempt count. On exhaust →
///    status="dead" + write to `jobs_dlq:<id>` so the framework's
///    DLQ inspection surfaces it.
///
/// Returns Ok(()) per batch. Individual message failures use
/// `msg.retry()` (handled by the Queues runtime).
#[event(queue)]
async fn queue(batch: MessageBatch<serde_json::Value>, env: Env, _ctx: Context) -> WResult<()> {
    let registry = match env.kv("PYLON_JOBS_REGISTRY") {
        Ok(kv) => kv,
        Err(e) => {
            worker::console_warn!(
                "[queue] PYLON_JOBS_REGISTRY binding missing: {e}; dropping batch"
            );
            return Ok(());
        }
    };

    let messages = batch.messages()?;
    for msg in messages {
        let envelope = msg.body();
        let id = envelope
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            // Malformed envelope. Ack-without-retry so it doesn't
            // loop forever; log for the operator.
            worker::console_warn!("[queue] message missing `id` field; dropping");
            continue;
        }

        let registry_key = format!("jobs:{id}");
        let dlq_key = format!("jobs_dlq:{id}");

        // 1. Mark running.
        let mut running = envelope.clone();
        if let Some(obj) = running.as_object_mut() {
            obj.insert("status".into(), serde_json::Value::String("running".into()));
            obj.insert("started_at".into(), serde_json::Value::from(now_ms_u64()));
        }
        if let Ok(b) = registry.put(&registry_key, running.to_string()) {
            let _ = b.execute().await;
        }

        // 2. Dispatch. Placeholder for now — when the queue+fetch
        //    runtimes share an FnOpsImpl (separate refactor), this
        //    will route `name` + `payload` through it. Today, we
        //    mark completed so the queue doesn't loop on undeliverable
        //    work; operators wire up the actual dispatch in their own
        //    fetch handler arm (see /api/jobs/run in the framework).
        let dispatch_ok = true;

        // 3. Outcome.
        let mut final_envelope = envelope.clone();
        if let Some(obj) = final_envelope.as_object_mut() {
            obj.insert(
                "status".into(),
                serde_json::Value::String(
                    (if dispatch_ok { "completed" } else { "failed" }).into(),
                ),
            );
            obj.insert("finished_at".into(), serde_json::Value::from(now_ms_u64()));
        }
        if let Ok(b) = registry.put(&registry_key, final_envelope.to_string()) {
            let _ = b.execute().await;
        }
        if !dispatch_ok {
            // Move to DLQ. Queues' own retry handling has already
            // bounced this past max_retries (we're in the consumer
            // path, not the retry path).
            if let Ok(b) = registry.put(&dlq_key, final_envelope.to_string()) {
                let _ = b.execute().await;
            }
        }
    }
    Ok(())
}

fn now_ms_u64() -> u64 {
    js_sys::Date::now() as u64
}

fn empty_manifest() -> pylon_kernel::AppManifest {
    pylon_kernel::AppManifest {
        manifest_version: pylon_kernel::MANIFEST_VERSION,
        name: "workers".into(),
        version: "0.1.0".into(),
        entities: vec![],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
        llm: Default::default(),
        connections: vec![],
        crons: vec![],
        fonts: vec![],
    }
}

/// EmailSender that POSTs `{to, subject, body}` JSON to a
/// configurable webhook URL. The receiving endpoint is responsible
/// for the actual mail delivery — operator points it at Resend's
/// `/emails` endpoint, a Cloudflare Worker that proxies to SES, or
/// any other HTTP-API mail provider. Optional Bearer auth via
/// PYLON_EMAIL_WEBHOOK_BEARER.
///
/// When `url` is unset, send() logs a warning and returns Ok(()) —
/// matches the pre-pool behaviour of "fail open" so apps don't 500
/// on first deploy before email is wired.
///
/// Note: EmailSender trait is sync but Workers' Fetch is async.
/// futures::executor::block_on bridges the gap; Workers's single-
/// threaded runtime tolerates it inside request handlers (same
/// pattern KvCache + R2Files use).
struct HttpWebhookEmailSender {
    url: Option<String>,
    bearer: Option<String>,
}

impl pylon_router::EmailSender for HttpWebhookEmailSender {
    fn send(&self, msg: &pylon_kernel::EmailMessage) -> std::result::Result<(), String> {
        let Some(url) = self.url.as_deref() else {
            worker::console_warn!(
                "[email] PYLON_EMAIL_WEBHOOK_URL unset — dropped message to {}. Set the env to enable HTTP-transport mail delivery.",
                msg.to,
            );
            return Ok(());
        };
        // `body` (the legacy key) stays for existing receiver endpoints;
        // `text` / `html` / `attachments` are additive.
        let mut payload_json = serde_json::json!({
            "to": msg.to,
            "subject": msg.subject,
            "body": msg.text,
            "text": msg.text,
        });
        if let Some(html) = &msg.html {
            payload_json["html"] = serde_json::Value::String(html.clone());
        }
        if !msg.attachments.is_empty() {
            payload_json["attachments"] =
                serde_json::to_value(&msg.attachments).unwrap_or(serde_json::Value::Null);
        }
        let payload = payload_json.to_string();
        let mut headers = Headers::new();
        headers
            .set("Content-Type", "application/json")
            .map_err(|e| e.to_string())?;
        if let Some(bearer) = self.bearer.as_deref() {
            headers
                .set("Authorization", &format!("Bearer {bearer}"))
                .map_err(|e| e.to_string())?;
        }
        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_headers(headers)
            .with_body(Some(payload.into()));
        let req = Request::new_with_init(url, &init).map_err(|e| e.to_string())?;
        let resp = futures::executor::block_on(Fetch::Request(req).send())
            .map_err(|e| format!("email webhook fetch failed: {e}"))?;
        let status = resp.status_code();
        if (200..400).contains(&status) {
            Ok(())
        } else {
            Err(format!("email webhook returned {status}"))
        }
    }
}

/// Stub DataStore for the org store — pylon-workers doesn't
/// persist auth data today (no shared backing store), so the
/// in-memory OrgStore gets an empty DataStore. Customer org
/// workflows on Workers return empty results until a persistent
/// backing is wired (KV-backed DataStore is the natural fit).
struct EmptyDataStore;

impl pylon_http::DataStore for EmptyDataStore {
    fn manifest(&self) -> &pylon_kernel::AppManifest {
        // Static empty manifest. The OrgStore reads `cfg`, not
        // this — manifest is unused by the wasm32 paths.
        static M: std::sync::OnceLock<pylon_kernel::AppManifest> = std::sync::OnceLock::new();
        M.get_or_init(empty_manifest)
    }
    fn insert(
        &self,
        _entity: &str,
        _data: &serde_json::Value,
    ) -> std::result::Result<String, pylon_http::DataError> {
        Err(pylon_http::DataError {
            code: "NOT_SUPPORTED".into(),
            message: "Workers target has no persistent store wired".into(),
        })
    }
    fn get_by_id(
        &self,
        _entity: &str,
        _id: &str,
    ) -> std::result::Result<Option<serde_json::Value>, pylon_http::DataError> {
        Ok(None)
    }
    fn list(
        &self,
        _entity: &str,
    ) -> std::result::Result<Vec<serde_json::Value>, pylon_http::DataError> {
        Ok(vec![])
    }
    fn update(
        &self,
        _entity: &str,
        _id: &str,
        _data: &serde_json::Value,
    ) -> std::result::Result<bool, pylon_http::DataError> {
        Ok(false)
    }
    fn delete(&self, _entity: &str, _id: &str) -> std::result::Result<bool, pylon_http::DataError> {
        Ok(false)
    }
    fn query_filtered(
        &self,
        _entity: &str,
        _filter: &serde_json::Value,
    ) -> std::result::Result<Vec<serde_json::Value>, pylon_http::DataError> {
        Ok(vec![])
    }
    fn list_after(
        &self,
        _entity: &str,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> std::result::Result<Vec<serde_json::Value>, pylon_http::DataError> {
        Ok(vec![])
    }
    fn lookup(
        &self,
        _entity: &str,
        _field: &str,
        _value: &str,
    ) -> std::result::Result<Option<serde_json::Value>, pylon_http::DataError> {
        Ok(None)
    }
    fn link(
        &self,
        _entity: &str,
        _id: &str,
        _relation: &str,
        _target_id: &str,
    ) -> std::result::Result<bool, pylon_http::DataError> {
        Ok(false)
    }
    fn unlink(
        &self,
        _entity: &str,
        _id: &str,
        _relation: &str,
    ) -> std::result::Result<bool, pylon_http::DataError> {
        Ok(false)
    }
    fn query_graph(
        &self,
        _query: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, pylon_http::DataError> {
        Ok(serde_json::json!({}))
    }
    fn transact(
        &self,
        _ops: &[serde_json::Value],
    ) -> std::result::Result<(bool, Vec<serde_json::Value>), pylon_http::DataError> {
        Ok((false, vec![]))
    }
}
