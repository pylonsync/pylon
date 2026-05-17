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
    event, Context, D1Database, D1Type, Env, Headers, Request, Response, Result as WResult,
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
        let rows = result.results::<serde_json::Value>().map_err(|e| e.to_string())?;
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
            // Strings + arrays + objects all serialize as TEXT —
            // the JSON shape is what gets stored. Caller is
            // responsible for parsing on read.
            other => D1Type::Text(
                // D1Type::Text borrows; we need 'static so leak.
                // This path is called per-query; allocations are
                // tiny relative to the network round-trip cost.
                Box::leak(serde_json::to_string(other).unwrap_or_default().into_boxed_str()),
            ),
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
    let email = NoopEmailSender;

    // Optional real bindings — if the operator declared them in
    // wrangler.toml, plug in the KV / R2 adapters; otherwise fall
    // back to NoopAll's typed-503 stubs so missing bindings
    // surface as KV_BINDING_REQUIRED / R2_BINDING_REQUIRED rather
    // than mysteriously failing.
    let kv_cache_opt = env.kv("PYLON_CACHE").ok().map(crate::KvCache::new);
    let r2_files_opt = env.bucket("PYLON_FILES").ok().map(crate::R2Files::new);
    let cache_ref: &dyn pylon_router::CacheOps = match kv_cache_opt.as_ref() {
        Some(c) => c,
        None => &noop,
    };
    let files_ref: &dyn pylon_router::FileOps = match r2_files_opt.as_ref() {
        Some(f) => f,
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
        rooms: &noop,
        cache: cache_ref,
        pubsub: &noop,
        jobs: &noop,
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
    headers.set("Access-Control-Allow-Headers", "Content-Type, Authorization")?;

    Ok(Response::ok(response_body)?
        .with_status(status)
        .with_headers(headers))
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
    }
}

/// Email sender stub. Workers EmailSender wiring lands when we
/// pick the http-transport contract for the env (Resend / Postmark
/// / plain HTTP POST) — for now, returns Ok(()) so apps that call
/// ctx.email.send don't 500. Logged at WARN so the gap is visible.
struct NoopEmailSender;

impl pylon_router::EmailSender for NoopEmailSender {
    fn send(&self, to: &str, _subject: &str, _body: &str) -> std::result::Result<(), String> {
        worker::console_warn!(
            "[email] Workers target has no email transport configured — dropped message to {to}. Wire a fetch-based EmailSender to an HTTP transport (Resend/Postmark/SES) to enable.",
        );
        Ok(())
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
    fn delete(
        &self,
        _entity: &str,
        _id: &str,
    ) -> std::result::Result<bool, pylon_http::DataError> {
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
