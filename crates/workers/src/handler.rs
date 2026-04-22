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

use statecraft_http::HttpMethod;
use statecraft_router::{route, RouterContext};
use worker::*;

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
    fn execute(&self, sql: &str, params: &[serde_json::Value]) -> Result<u64, String> {
        // Workers is single-threaded and async. We use `block_on` through
        // futures::executor since the Workers runtime allows it in request handlers.
        let stmt = self.db.prepare(sql);
        let bound = stmt
            .bind_refs(&params_to_js(params))
            .map_err(|e| e.to_string())?;

        let result = futures::executor::block_on(bound.run()).map_err(|e| e.to_string())?;
        Ok(result.meta().ok().flatten().and_then(|m| m.changes).unwrap_or(0) as u64)
    }

    fn query(&self, sql: &str, params: &[serde_json::Value]) -> Result<Vec<serde_json::Value>, String> {
        let stmt = self.db.prepare(sql);
        let bound = stmt
            .bind_refs(&params_to_js(params))
            .map_err(|e| e.to_string())?;

        let result = futures::executor::block_on(bound.all()).map_err(|e| e.to_string())?;
        let rows = result.results::<serde_json::Value>().map_err(|e| e.to_string())?;
        Ok(rows)
    }
}

fn params_to_js(params: &[serde_json::Value]) -> Vec<wasm_bindgen::JsValue> {
    params
        .iter()
        .map(|v| serde_wasm_bindgen::to_value(v).unwrap_or(wasm_bindgen::JsValue::NULL))
        .collect()
}

// ---------------------------------------------------------------------------
// Fetch handler
// ---------------------------------------------------------------------------

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let method = HttpMethod::from_str(&req.method().to_string());
    let url = req.path();
    let body = req.text().await.unwrap_or_default();

    let auth_token = req
        .headers()
        .get("Authorization")?
        .and_then(|v| v.strip_prefix("Bearer ").map(String::from));

    // Load manifest from a KV/env binding.
    let manifest_json = env
        .var("STATECRAFT_MANIFEST_JSON")
        .map(|v| v.to_string())
        .unwrap_or_else(|_| "{}".into());
    let manifest: statecraft_core::AppManifest =
        serde_json::from_str(&manifest_json).unwrap_or_else(|_| empty_manifest());

    let d1 = env.d1("STATECRAFT_DB")?;
    let executor = WorkerD1Executor::new(d1);
    let store = D1DataStore::new(executor, manifest.clone());

    let session_store = statecraft_auth::SessionStore::new();
    let magic_codes = statecraft_auth::MagicCodeStore::new();
    let oauth_state = statecraft_auth::OAuthStateStore::new();
    let policy_engine = statecraft_policy::PolicyEngine::from_manifest(&manifest);
    let change_log = statecraft_sync::ChangeLog::new();
    let auth_ctx = session_store.resolve(auth_token.as_deref());
    let noop = NoopAll::new(&manifest);
    let email = NoopEmailSender;

    let ctx = RouterContext {
        store: &store,
        session_store: &session_store,
        magic_codes: &magic_codes,
        oauth_state: &oauth_state,
        policy_engine: &policy_engine,
        change_log: &change_log,
        notifier: &statecraft_router::NoopNotifier,
        rooms: &noop,
        cache: &noop,
        pubsub: &noop,
        jobs: &noop,
        scheduler: &noop,
        workflows: &noop,
        files: &noop,
        openapi: &noop,
        functions: None,
        email: &email,
        shards: None,
        plugin_hooks: &statecraft_router::NoopPluginHooks,
        auth_ctx: &auth_ctx,
        is_dev: false,
        // Workers doesn't forward request headers into the router yet —
        // webhook endpoints won't get signature headers here. Populate
        // when adding webhook support to the Workers target.
        request_headers: &[],
    };

    let (status, response_body, _ct) =
        route(&ctx, method, &url, &body, auth_token.as_deref());

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

fn empty_manifest() -> statecraft_core::AppManifest {
    statecraft_core::AppManifest {
        manifest_version: statecraft_core::MANIFEST_VERSION,
        name: "workers".into(),
        version: "0.1.0".into(),
        entities: vec![],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
    }
}

struct NoopEmailSender;

impl statecraft_router::EmailSender for NoopEmailSender {
    fn send(&self, _to: &str, _subject: &str, _body: &str) -> std::result::Result<(), String> {
        // Workers env can configure email via their own transport; a follow-up
        // will add a Workers-compatible HTTP transport using `fetch`.
        Ok(())
    }
}
