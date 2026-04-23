//! Platform-agnostic HTTP router for pylon.
//!
//! This crate contains the pure routing logic that maps HTTP requests to
//! data store operations. It has no I/O dependencies — no `tiny_http`,
//! no `tungstenite`, no `rusqlite`. It works with any [`DataStore`]
//! implementation (SQLite Runtime, Cloudflare D1, etc.).

use pylon_auth::{AuthContext, MagicCodeStore, OAuthStateStore, SessionStore};
use pylon_http::{DataError, DataStore, HttpMethod};
use pylon_policy::PolicyEngine;
use pylon_sync::{ChangeKind, ChangeLog, SyncCursor};

// ---------------------------------------------------------------------------
// ChangeNotifier — abstraction over WS/SSE broadcast
// ---------------------------------------------------------------------------

/// Receives change notifications for real-time sync.
///
/// On the self-hosted server this broadcasts to WebSocket + SSE hubs.
/// On Workers this can be a no-op or post to a Durable Object.
pub trait ChangeNotifier: Send + Sync {
    fn notify(&self, event: &pylon_sync::ChangeEvent);
    fn notify_presence(&self, json: &str);
}

/// No-op notifier for platforms without real-time push.
pub struct NoopNotifier;

impl ChangeNotifier for NoopNotifier {
    fn notify(&self, _event: &pylon_sync::ChangeEvent) {}
    fn notify_presence(&self, _json: &str) {}
}

// ---------------------------------------------------------------------------
// CacheOps / PubSubOps / JobOps / SchedulerOps / WorkflowOps
// — thin traits so the router doesn't depend on concrete impls
// ---------------------------------------------------------------------------

/// Cache operations used by the router.
pub trait CacheOps: Send + Sync {
    fn handle_command(&self, body: &str) -> (u16, String);
    fn handle_get(&self, key: &str) -> (u16, String);
    fn handle_delete(&self, key: &str) -> (u16, String);
}

/// Pub/Sub operations used by the router.
pub trait PubSubOps: Send + Sync {
    fn handle_publish(&self, body: &str) -> (u16, String);
    fn handle_channels(&self) -> (u16, String);
    fn handle_history(&self, channel: &str, url: &str) -> (u16, String);
}

/// Room operations used by the router.
pub trait RoomOps: Send + Sync {
    fn join(
        &self,
        room: &str,
        user_id: &str,
        data: Option<serde_json::Value>,
    ) -> Result<(serde_json::Value, serde_json::Value), DataError>;
    fn leave(&self, room: &str, user_id: &str) -> Option<serde_json::Value>;
    fn set_presence(
        &self,
        room: &str,
        user_id: &str,
        data: serde_json::Value,
    ) -> Option<serde_json::Value>;
    fn broadcast(
        &self,
        room: &str,
        sender: Option<&str>,
        topic: &str,
        data: serde_json::Value,
    ) -> Option<serde_json::Value>;
    fn list_rooms(&self) -> Vec<String>;
    fn room_size(&self, name: &str) -> usize;
    fn members(&self, name: &str) -> Vec<serde_json::Value>;
}

/// Job queue operations used by the router.
pub trait JobOps: Send + Sync {
    fn enqueue(
        &self,
        name: &str,
        payload: serde_json::Value,
        priority: &str,
        delay_secs: u64,
        max_retries: u32,
        queue: &str,
    ) -> String;
    fn stats(&self) -> serde_json::Value;
    fn dead_letters(&self) -> serde_json::Value;
    fn retry_dead(&self, id: &str) -> bool;
    fn list_jobs(
        &self,
        status: Option<&str>,
        queue: Option<&str>,
        limit: usize,
    ) -> serde_json::Value;
    fn get_job(&self, id: &str) -> Option<serde_json::Value>;
}

/// Scheduler operations used by the router.
pub trait SchedulerOps: Send + Sync {
    fn list_tasks(&self) -> serde_json::Value;
    fn trigger(&self, name: &str) -> bool;
}

/// Workflow engine operations used by the router.
pub trait WorkflowOps: Send + Sync {
    fn definitions(&self) -> serde_json::Value;
    fn start(&self, name: &str, input: serde_json::Value) -> Result<String, String>;
    fn list(&self, status_filter: Option<&str>) -> serde_json::Value;
    fn get(&self, id: &str) -> Option<serde_json::Value>;
    fn advance(&self, id: &str) -> Result<String, String>;
    fn send_event(&self, id: &str, event: &str, data: serde_json::Value) -> Result<(), String>;
    fn cancel(&self, id: &str) -> Result<(), String>;
}

/// File storage operations used by the router.
pub trait FileOps: Send + Sync {
    fn upload(&self, body: &str) -> (u16, String);
    fn get_file(&self, id: &str) -> (u16, String);
}

/// Sends emails (magic codes, invitations, etc.).
pub trait EmailSender: Send + Sync {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String>;
}

/// Access to sharded real-time simulations (game matches, MMO zones, etc.).
pub trait ShardOps: Send + Sync {
    /// Look up an existing shard by ID.
    fn get_shard(&self, id: &str) -> Option<std::sync::Arc<dyn pylon_realtime::DynShard>>;

    /// List IDs of active shards.
    fn list_shards(&self) -> Vec<String>;

    /// Number of active shards.
    fn shard_count(&self) -> usize;
}

/// Generates the OpenAPI spec JSON string for the manifest.
pub trait OpenApiGenerator: Send + Sync {
    fn generate(&self, base_url: &str) -> String;
}

/// Plugin CRUD lifecycle hooks used by the router.
///
/// Wired up so that `POST/PATCH/DELETE /api/entities/...` triggers registered
/// plugin before_/after_ hooks (audit log, search indexing, webhooks,
/// versioning, validation, timestamps, slugify). Previously the router only
/// ran `on_request` + custom routes, silently bypassing CRUD hooks — which
/// meant security-relevant plugins (validation, audit_log) didn't apply to
/// the primary write path.
///
/// `before_insert/update` receive a mutable `data` so plugins can inject
/// fields (timestamps, slugs). A returned `Err` rejects the write with the
/// given status + message and no data is touched.
pub trait PluginHookOps: Send + Sync {
    fn before_insert(
        &self,
        entity: &str,
        data: &mut serde_json::Value,
        auth: &AuthContext,
    ) -> Result<(), (u16, String, String)>;

    fn after_insert(&self, entity: &str, id: &str, data: &serde_json::Value, auth: &AuthContext);

    fn before_update(
        &self,
        entity: &str,
        id: &str,
        data: &mut serde_json::Value,
        auth: &AuthContext,
    ) -> Result<(), (u16, String, String)>;

    fn after_update(&self, entity: &str, id: &str, data: &serde_json::Value, auth: &AuthContext);

    fn before_delete(
        &self,
        entity: &str,
        id: &str,
        auth: &AuthContext,
    ) -> Result<(), (u16, String, String)>;

    fn after_delete(&self, entity: &str, id: &str, auth: &AuthContext);
}

/// No-op plugin hooks for platforms or tests without a registry.
pub struct NoopPluginHooks;

impl PluginHookOps for NoopPluginHooks {
    fn before_insert(
        &self,
        _entity: &str,
        _data: &mut serde_json::Value,
        _auth: &AuthContext,
    ) -> Result<(), (u16, String, String)> {
        Ok(())
    }
    fn after_insert(
        &self,
        _entity: &str,
        _id: &str,
        _data: &serde_json::Value,
        _auth: &AuthContext,
    ) {
    }
    fn before_update(
        &self,
        _entity: &str,
        _id: &str,
        _data: &mut serde_json::Value,
        _auth: &AuthContext,
    ) -> Result<(), (u16, String, String)> {
        Ok(())
    }
    fn after_update(
        &self,
        _entity: &str,
        _id: &str,
        _data: &serde_json::Value,
        _auth: &AuthContext,
    ) {
    }
    fn before_delete(
        &self,
        _entity: &str,
        _id: &str,
        _auth: &AuthContext,
    ) -> Result<(), (u16, String, String)> {
        Ok(())
    }
    fn after_delete(&self, _entity: &str, _id: &str, _auth: &AuthContext) {}
}

/// TypeScript function operations used by the router.
///
/// Implementations manage transaction semantics: mutations run under the
/// write lock with BEGIN/COMMIT/ROLLBACK, queries use the read pool,
/// actions run without transactions.
pub trait FnOps: Send + Sync {
    /// Look up a registered function.
    fn get_fn(&self, name: &str) -> Option<pylon_functions::registry::FnDef>;

    /// List all registered functions.
    fn list_fns(&self) -> Vec<pylon_functions::registry::FnDef>;

    /// Execute a function. For streaming responses, `on_stream` is called for
    /// each chunk as it arrives from the function handler.
    ///
    /// `request` is populated when the function is invoked via a custom HTTP
    /// route (`defineRoute` binding). It carries the raw request metadata
    /// (method, path, headers, body bytes) so actions can verify webhook
    /// signatures. Pass `None` for programmatic invocations (runAction,
    /// scheduled jobs, admin dashboard).
    fn call(
        &self,
        fn_name: &str,
        args: serde_json::Value,
        auth: pylon_functions::protocol::AuthInfo,
        on_stream: Option<Box<dyn FnMut(&str) + Send>>,
        request: Option<pylon_functions::protocol::RequestInfo>,
    ) -> Result<
        (serde_json::Value, pylon_functions::trace::FnTrace),
        pylon_functions::runner::FnCallError,
    >;

    /// Recent traces for observability (newest first).
    fn recent_traces(&self, limit: usize) -> Vec<pylon_functions::trace::FnTrace>;

    /// Check whether the caller is allowed to invoke this function right now.
    ///
    /// Returns `Ok(())` if allowed, or `Err(retry_after_secs)` if the caller
    /// is over the per-function quota. Default impl is permissive — backends
    /// that don't enforce per-function limits don't need to implement it.
    /// `identity` is a stable string for the caller (user id, session id, or
    /// IP) used as the rate-limit key.
    fn check_rate_limit(&self, _fn_name: &str, _identity: &str) -> Result<(), u64> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// RouterContext — bundles all dependencies for a single request
// ---------------------------------------------------------------------------

pub struct RouterContext<'a> {
    pub store: &'a dyn DataStore,
    pub session_store: &'a SessionStore,
    pub magic_codes: &'a MagicCodeStore,
    pub oauth_state: &'a OAuthStateStore,
    pub policy_engine: &'a PolicyEngine,
    pub change_log: &'a ChangeLog,
    pub notifier: &'a dyn ChangeNotifier,
    pub rooms: &'a dyn RoomOps,
    pub cache: &'a dyn CacheOps,
    pub pubsub: &'a dyn PubSubOps,
    pub jobs: &'a dyn JobOps,
    pub scheduler: &'a dyn SchedulerOps,
    pub workflows: &'a dyn WorkflowOps,
    pub files: &'a dyn FileOps,
    pub openapi: &'a dyn OpenApiGenerator,
    pub functions: Option<&'a dyn FnOps>,
    pub email: &'a dyn EmailSender,
    pub shards: Option<&'a dyn ShardOps>,
    pub plugin_hooks: &'a dyn PluginHookOps,
    pub auth_ctx: &'a AuthContext,
    pub is_dev: bool,
    /// Raw HTTP request headers (lowercased names). Used by the webhook
    /// action endpoint to pass the exact signing-relevant headers through
    /// to TypeScript actions. Empty slice on platforms that don't forward
    /// headers (e.g. internal calls).
    pub request_headers: &'a [(String, String)],
}

// ---------------------------------------------------------------------------
// route() — the platform-agnostic request router
// ---------------------------------------------------------------------------

/// Route an HTTP request to the appropriate handler.
///
/// Returns `(status_code, response_body, content_type)`.
pub fn route(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    auth_token: Option<&str>,
) -> (u16, String, &'static str) {
    let (status, body) = route_inner(ctx, method, url, body, auth_token);
    (status, body, "application/json")
}

fn route_inner(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    auth_token: Option<&str>,
) -> (u16, String) {
    // CORS preflight
    if method == HttpMethod::Options {
        return (204, String::new());
    }

    // GET /api/manifest
    if url == "/api/manifest" && method == HttpMethod::Get {
        return (
            200,
            serde_json::to_string(ctx.store.manifest()).unwrap_or_else(|_| "{}".into()),
        );
    }

    // GET /api/openapi.json
    if url == "/api/openapi.json" && method == HttpMethod::Get {
        return (200, ctx.openapi.generate(""));
    }

    // -----------------------------------------------------------------------
    // Auth routes
    // -----------------------------------------------------------------------

    // POST /api/auth/session
    //
    // Mints a session for an arbitrary user_id. This is a privileged operation
    // — there is NO credential check here, only an admin/dev gate. Production
    // code must go through `/api/auth/magic/verify` or the OAuth callback.
    // Historically this route was ungated and any caller could become any
    // user. Now: dev mode OR admin token required.
    if url == "/api/auth/session" && method == HttpMethod::Post {
        if !ctx.is_dev && !ctx.auth_ctx.is_admin {
            return (
                403,
                json_error(
                    "FORBIDDEN",
                    "/api/auth/session requires admin auth in non-dev mode",
                ),
            );
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let user_id = match data.get("user_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return (400, json_error("MISSING_USER_ID", "user_id is required")),
        };
        let session = ctx.session_store.create(user_id);
        return (
            201,
            serde_json::json!({"token": session.token, "user_id": session.user_id}).to_string(),
        );
    }

    // GET /api/auth/me
    if url == "/api/auth/me" && method == HttpMethod::Get {
        let resolved = ctx.session_store.resolve(auth_token);
        return (
            200,
            serde_json::to_string(&resolved).unwrap_or_else(|_| "{}".into()),
        );
    }

    // POST /api/auth/guest
    if url == "/api/auth/guest" && method == HttpMethod::Post {
        let session = ctx.session_store.create_guest();
        return (
            201,
            serde_json::json!({"token": session.token, "user_id": session.user_id, "guest": true})
                .to_string(),
        );
    }

    // POST /api/auth/upgrade
    //
    // Swap a guest session's anonymous id for a real user id. Same hole as
    // /api/auth/session if ungated: a caller holding a guest token can
    // upgrade to anyone. Gate: admin auth, or dev mode, with the same
    // rationale as session mint. Real upgrade should flow through magic-code
    // verify or OAuth callback, which consume the previous guest token and
    // issue a fresh user token server-side.
    if url == "/api/auth/upgrade" && method == HttpMethod::Post {
        if !ctx.is_dev && !ctx.auth_ctx.is_admin {
            return (
                403,
                json_error(
                    "FORBIDDEN",
                    "/api/auth/upgrade requires admin auth in non-dev mode",
                ),
            );
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let user_id = match data.get("user_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => return (400, json_error("MISSING_USER_ID", "user_id is required")),
        };
        if let Some(token) = auth_token {
            if ctx.session_store.upgrade(token, user_id.clone()) {
                return (
                    200,
                    serde_json::json!({"upgraded": true, "user_id": user_id}).to_string(),
                );
            }
        }
        return (
            400,
            json_error("UPGRADE_FAILED", "No valid session to upgrade"),
        );
    }

    // POST /api/auth/select-org
    //
    // Switch the caller's active tenant (organization). The server does a
    // membership check against OrgMember before committing — a client can't
    // impersonate an org it doesn't belong to. Pass `{ orgId: null }` to
    // leave all orgs (back to the login lobby).
    if url == "/api/auth/select-org" && method == HttpMethod::Post {
        let token = match auth_token {
            Some(t) => t,
            None => return (401, json_error("UNAUTHENTICATED", "missing bearer token")),
        };
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(id) => id,
            None => return (401, json_error("UNAUTHENTICATED", "anonymous session")),
        };
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                );
            }
        };
        let target = data.get("orgId").and_then(|v| {
            if v.is_null() {
                Some(String::new())
            } else {
                v.as_str().map(String::from)
            }
        });
        let target = match target {
            Some(t) => t,
            None => {
                return (
                    400,
                    json_error("MISSING_ORG_ID", "orgId is required (or null)"),
                )
            }
        };
        if target.is_empty() {
            // Clear the active org — the user is dropping out of all tenants.
            ctx.session_store.set_tenant(token, None);
            return (200, serde_json::json!({"tenantId": null}).to_string());
        }
        // Look up an OrgMember row matching this user + target org.
        let filter = serde_json::json!({ "userId": user_id, "orgId": &target });
        match ctx.store.query_filtered("OrgMember", &filter) {
            Ok(rows) if !rows.is_empty() => {
                ctx.session_store.set_tenant(token, Some(target.clone()));
                return (200, serde_json::json!({"tenantId": target}).to_string());
            }
            Ok(_) => {
                return (
                    403,
                    json_error(
                        "NOT_A_MEMBER",
                        "you are not a member of the target organization",
                    ),
                );
            }
            Err(e) => {
                return (
                    500,
                    json_error_safe("LOOKUP_FAILED", "could not verify membership", &e.message),
                );
            }
        }
    }

    // POST /api/auth/magic/send
    if url == "/api/auth/magic/send" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let email = match data.get("email").and_then(|v| v.as_str()) {
            Some(e) => e.to_string(),
            None => return (400, json_error("MISSING_EMAIL", "email is required")),
        };
        let code = match ctx.magic_codes.try_create(&email) {
            Ok(c) => c,
            Err(pylon_auth::MagicCodeError::Throttled { retry_after_secs }) => {
                return (
                    429,
                    json_error_with_hint(
                        "RATE_LIMITED",
                        "A sign-in code was requested too recently.",
                        &format!("Try again in {retry_after_secs} seconds."),
                    ),
                );
            }
            Err(e) => {
                return (
                    500,
                    json_error(
                        "EMAIL_SEND_FAILED",
                        &format!("Could not issue code: {:?}", e),
                    ),
                );
            }
        };
        let subject = "Your sign-in code";
        let body_text =
            format!("Your sign-in code is: {code}\n\nThis code will expire in 10 minutes.");
        if let Err(e) = ctx.email.send(&email, subject, &body_text) {
            if !ctx.is_dev {
                tracing::warn!("[email] Failed to send magic code to {email}: {e}");
                return (
                    500,
                    json_error("EMAIL_SEND_FAILED", "Could not send sign-in email"),
                );
            }
        }
        if ctx.is_dev {
            return (
                200,
                serde_json::json!({"sent": true, "email": email, "dev_code": code}).to_string(),
            );
        }
        return (
            200,
            serde_json::json!({"sent": true, "email": email}).to_string(),
        );
    }

    // POST /api/auth/magic/verify
    if url == "/api/auth/magic/verify" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let email = match data.get("email").and_then(|v| v.as_str()) {
            Some(e) => e,
            None => return (400, json_error("MISSING_EMAIL", "email is required")),
        };
        let code = match data.get("code").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return (400, json_error("MISSING_CODE", "code is required")),
        };
        match ctx.magic_codes.try_verify(email, code) {
            Ok(()) => {
                let user_id = match ctx.store.lookup("User", "email", email) {
                    Ok(Some(row)) => row["id"].as_str().unwrap_or("").to_string(),
                    _ => {
                        let now = format!(
                            "{}Z",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs()
                        );
                        ctx.store
                            .insert(
                                "User",
                                &serde_json::json!({"email": email, "displayName": email, "createdAt": now}),
                            )
                            .unwrap_or_else(|_| email.to_string())
                    }
                };
                let session = ctx.session_store.create(user_id.clone());
                return (
                    200,
                    serde_json::json!({"token": session.token, "user_id": user_id, "expires_at": session.expires_at}).to_string(),
                );
            }
            Err(pylon_auth::MagicCodeError::TooManyAttempts) => {
                return (
                    429,
                    json_error(
                        "RATE_LIMITED",
                        "Too many verification attempts. Request a new code.",
                    ),
                );
            }
            Err(_) => {}
        }
        return (401, json_error("INVALID_CODE", "Invalid or expired code"));
    }

    // GET /api/auth/providers
    if url == "/api/auth/providers" && method == HttpMethod::Get {
        let registry = pylon_auth::OAuthRegistry::from_env();
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
        return (
            200,
            serde_json::to_string(&providers).unwrap_or_else(|_| "[]".into()),
        );
    }

    // GET /api/auth/login/:provider
    if let Some(provider) = url.strip_prefix("/api/auth/login/") {
        let provider = provider.split('?').next().unwrap_or(provider);
        if method == HttpMethod::Get {
            let registry = pylon_auth::OAuthRegistry::from_env();
            if let Some(config) = registry.get(provider) {
                let state = ctx.oauth_state.create(provider);
                return (
                    200,
                    serde_json::json!({"redirect": config.auth_url_with_state(&state), "state": state}).to_string(),
                );
            }
            return (
                404,
                json_error_with_hint(
                    "PROVIDER_NOT_FOUND",
                    &format!("OAuth provider \"{provider}\" is not configured"),
                    "Set PYLON_OAUTH_GOOGLE_CLIENT_ID / PYLON_OAUTH_GITHUB_CLIENT_ID environment variables",
                ),
            );
        }
    }

    // POST /api/auth/callback/:provider — exchange authorization code for
    // a session. Accepts `{code, state}` (real OAuth flow) or a legacy
    // `{email, state}` shape where the client has already resolved the user
    // (kept for server-side testing and non-browser clients).
    if let Some(provider) = url.strip_prefix("/api/auth/callback/") {
        let provider = provider.split('?').next().unwrap_or(provider);
        if method == HttpMethod::Post {
            let data: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => {
                    return (
                        400,
                        json_error_safe(
                            "INVALID_JSON",
                            "Invalid request body",
                            &format!("Invalid JSON: {e}"),
                        ),
                    )
                }
            };

            // Validate CSRF state.
            let state = data.get("state").and_then(|v| v.as_str());
            match state {
                Some(s) if ctx.oauth_state.validate(s, provider) => {}
                _ => {
                    return (
                        403,
                        json_error(
                            "OAUTH_INVALID_STATE",
                            "Invalid or missing OAuth state parameter",
                        ),
                    )
                }
            }

            // Require a real OAuth authorization code — the provider must
            // vouch for the email. Previously a legacy `{email, state}` path
            // allowed any caller who could fetch a state token to mint a
            // session for any email, which is account takeover. That path is
            // gone except under PYLON_DEV_MODE=true for integration tests
            // that don't want to run a real provider.
            let code = data.get("code").and_then(|v| v.as_str());

            let (email, name) = if let Some(code) = code {
                let registry = pylon_auth::OAuthRegistry::from_env();
                let config = match registry.get(provider) {
                    Some(c) => c.clone(),
                    None => {
                        return (
                            404,
                            json_error(
                                "PROVIDER_NOT_FOUND",
                                &format!("OAuth provider \"{provider}\" not configured"),
                            ),
                        )
                    }
                };
                match config.exchange_code(code) {
                    Ok(access_token) => match config.fetch_userinfo(&access_token) {
                        Ok((e, n)) => (e, n),
                        Err(err) => {
                            return (
                                502,
                                json_error(
                                    "OAUTH_TOKEN_EXCHANGE_FAILED",
                                    &format!("userinfo fetch failed: {err}"),
                                ),
                            )
                        }
                    },
                    Err(err) => {
                        return (
                            502,
                            json_error(
                                "OAUTH_TOKEN_EXCHANGE_FAILED",
                                &format!("token exchange failed: {err}"),
                            ),
                        )
                    }
                }
            } else if ctx.is_dev {
                let explicit_email = data.get("email").and_then(|v| v.as_str());
                let explicit_name = data.get("name").and_then(|v| v.as_str());
                match explicit_email {
                    Some(e) => (e.to_string(), explicit_name.map(String::from)),
                    None => {
                        return (
                            400,
                            json_error(
                                "MISSING_FIELD",
                                "OAuth callback requires `code` (or `email` in dev mode)",
                            ),
                        )
                    }
                }
            } else {
                return (
                    400,
                    json_error(
                        "MISSING_FIELD",
                        "OAuth callback requires an authorization `code` from the provider",
                    ),
                );
            };

            let user_id = match ctx.store.lookup("User", "email", &email) {
                Ok(Some(row)) => row["id"].as_str().unwrap_or("").to_string(),
                _ => {
                    let display_name = name.as_deref().unwrap_or(&email);
                    let now = format!(
                        "{}Z",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                    );
                    ctx.store
                        .insert(
                            "User",
                            &serde_json::json!({"email": email, "displayName": display_name, "createdAt": now}),
                        )
                        .unwrap_or_else(|_| email.clone())
                }
            };
            let session = ctx.session_store.create(user_id.clone());
            return (
                200,
                serde_json::json!({
                    "token": session.token,
                    "user_id": user_id,
                    "provider": provider,
                    "expires_at": session.expires_at,
                })
                .to_string(),
            );
        }
    }

    // DELETE /api/auth/session
    if url == "/api/auth/session" && method == HttpMethod::Delete {
        if let Some(token) = auth_token {
            ctx.session_store.revoke(token);
        }
        return (200, serde_json::json!({"revoked": true}).to_string());
    }

    // POST /api/auth/refresh — exchange current token for a new one
    if url == "/api/auth/refresh" && method == HttpMethod::Post {
        let old = match auth_token {
            Some(t) => t,
            None => return (401, json_error("AUTH_REQUIRED", "No session to refresh")),
        };
        match ctx.session_store.refresh(old) {
            Some(session) => {
                return (
                    200,
                    serde_json::json!({
                        "token": session.token,
                        "user_id": session.user_id,
                        "expires_at": session.expires_at,
                    })
                    .to_string(),
                )
            }
            None => {
                return (
                    401,
                    json_error("SESSION_EXPIRED", "Session is expired or invalid"),
                )
            }
        }
    }

    // GET /api/auth/sessions — list current user's active sessions (no tokens)
    if url == "/api/auth/sessions" && method == HttpMethod::Get {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u,
            None => return (401, json_error("AUTH_REQUIRED", "Login required")),
        };
        let list = ctx.session_store.list_for_user(user_id);
        let sanitized: Vec<serde_json::Value> = list
            .iter()
            .map(|s| {
                serde_json::json!({
                    "token_prefix": &s.token[..s.token.len().min(8)],
                    "user_id": s.user_id,
                    "device": s.device,
                    "created_at": s.created_at,
                    "expires_at": s.expires_at,
                })
            })
            .collect();
        return (
            200,
            serde_json::to_string(&sanitized).unwrap_or_else(|_| "[]".into()),
        );
    }

    // DELETE /api/auth/sessions — revoke all sessions for current user
    if url == "/api/auth/sessions" && method == HttpMethod::Delete {
        let user_id = match ctx.auth_ctx.user_id.as_deref() {
            Some(u) => u,
            None => return (401, json_error("AUTH_REQUIRED", "Login required")),
        };
        let n = ctx.session_store.revoke_all_for_user(user_id);
        return (200, serde_json::json!({"revoked_count": n}).to_string());
    }

    // -----------------------------------------------------------------------
    // Sync API
    // -----------------------------------------------------------------------

    // GET /api/sync/pull
    if url.starts_with("/api/sync/pull") && method == HttpMethod::Get {
        let since: u64 = url
            .split("since=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        match ctx.change_log.pull(&SyncCursor { last_seq: since }, 100) {
            Ok(mut resp) => {
                // Filter changes through the read-policy fence. Previously a
                // caller could pull every mutation regardless of which entities
                // their policy permitted — a silent bypass of read gates. We
                // evaluate per change so row-level policies that depend on
                // `data` still get a chance to match (deletes pass `None`).
                resp.changes.retain(|ev| {
                    matches!(
                        ctx.policy_engine.check_entity_read(
                            &ev.entity,
                            ctx.auth_ctx,
                            ev.data.as_ref()
                        ),
                        pylon_policy::PolicyResult::Allowed
                    )
                });
                return (
                    200,
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                );
            }
            Err(pylon_sync::PullError::ResyncRequired { oldest_seq, .. }) => {
                // Surfacing this to the client is the whole point of the new
                // error variant — previously the server silently skipped
                // the evicted range. Response shape is stable JSON so
                // clients can parse it without changing request shape.
                return (
                    410,
                    serde_json::json!({
                        "error": {
                            "code": "RESYNC_REQUIRED",
                            "message": format!(
                                "cursor last_seq={since} is older than the oldest retained seq={oldest_seq}; client must re-sync"
                            ),
                            "oldest_seq": oldest_seq,
                        }
                    })
                    .to_string(),
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // GDPR-style data-subject endpoints (admin-gated)
    // -----------------------------------------------------------------------
    //
    // `/api/admin/users/:id/export` and `/api/admin/users/:id/purge` satisfy
    // Articles 15 (access) and 17 (erasure). Both scan the manifest for
    // entities that reference users via conventional field names and touch
    // every row matching the target user_id. Admin-only — these expose
    // every row tied to a user across the whole database.

    if let Some(tail) = url.strip_prefix("/api/admin/users/") {
        let tail = tail.split('?').next().unwrap_or(tail);
        if let Some((user_id, action)) = tail.split_once('/') {
            if !user_id.is_empty() {
                // Export
                if action == "export" && method == HttpMethod::Post {
                    if let Some(err) = require_admin(ctx) {
                        return err;
                    }
                    return gdpr_export(ctx, user_id);
                }
                // Purge (hard delete + session revoke)
                if action == "purge" && method == HttpMethod::Delete {
                    if let Some(err) = require_admin(ctx) {
                        return err;
                    }
                    return gdpr_purge(ctx, user_id);
                }
            }
        }
    }

    // POST /api/sync/push
    if url == "/api/sync/push" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return err;
        }
        let push_req: pylon_sync::PushRequest = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };

        let mut applied = 0u32;
        let mut errors: Vec<String> = Vec::new();
        let mut deduped = 0u32;

        for change in &push_req.changes {
            // Idempotency: if the client minted an op_id and we've already
            // processed it, skip re-apply. This makes at-least-once delivery
            // from the client safe — retries after a timeout no longer create
            // duplicate writes. Clients without op_ids get the old behavior.
            if let Some(ref op_id) = change.op_id {
                if ctx.change_log.has_seen_op_id(op_id) {
                    deduped += 1;
                    continue;
                }
            }
            match change.kind {
                ChangeKind::Insert => {
                    if let Some(ref data) = change.data {
                        match ctx.store.insert(&change.entity, data) {
                            Ok(id) => {
                                ctx.change_log.append(
                                    &change.entity,
                                    &id,
                                    ChangeKind::Insert,
                                    change.data.clone(),
                                );
                                applied += 1;
                            }
                            Err(e) => {
                                errors.push(format!("insert {}: {}", change.entity, e.message))
                            }
                        }
                    }
                }
                ChangeKind::Update => {
                    if let Some(ref data) = change.data {
                        match ctx.store.update(&change.entity, &change.row_id, data) {
                            Ok(_) => {
                                ctx.change_log.append(
                                    &change.entity,
                                    &change.row_id,
                                    ChangeKind::Update,
                                    change.data.clone(),
                                );
                                applied += 1;
                            }
                            Err(e) => errors.push(format!(
                                "update {}/{}: {}",
                                change.entity, change.row_id, e.message
                            )),
                        }
                    }
                }
                ChangeKind::Delete => match ctx.store.delete(&change.entity, &change.row_id) {
                    Ok(_) => {
                        ctx.change_log.append(
                            &change.entity,
                            &change.row_id,
                            ChangeKind::Delete,
                            None,
                        );
                        applied += 1;
                    }
                    Err(e) => errors.push(format!(
                        "delete {}/{}: {}",
                        change.entity, change.row_id, e.message
                    )),
                },
            }
        }

        // Register processed op_ids AFTER the fact. The rule: remember the
        // op_id only if no error was recorded for that specific change.
        // Failed applies must NOT be marked seen or a retry will be falsely
        // treated as a replay and skipped forever.
        //
        // (We walk the changes + the errors vec together by zipping positions.
        // Errors carry enough context to correlate back; we use the simpler
        // "no error pushed this iteration" approximation below.)
        for change in &push_req.changes {
            if let Some(ref op_id) = change.op_id {
                let mention = format!(" {}", change.row_id);
                if !errors
                    .iter()
                    .any(|e| e.contains(&change.entity) && e.contains(&mention))
                {
                    ctx.change_log.remember_op_id(op_id);
                }
            }
        }

        return (
            200,
            serde_json::json!({
                "applied": applied,
                "deduped": deduped,
                "errors": errors,
                "cursor": {"last_seq": ctx.change_log.len()}
            })
            .to_string(),
        );
    }

    // -----------------------------------------------------------------------
    // Rooms API
    // -----------------------------------------------------------------------

    if url == "/api/rooms/join" && method == HttpMethod::Post {
        if let Some(err) = require_auth(ctx) {
            return err;
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let room = match data.get("room").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return (400, json_error("MISSING_ROOM", "room is required")),
        };
        // Trust auth_ctx FIRST so a client can't impersonate another user by
        // putting someone else's id in the body. Admins may still override
        // (useful for server-to-server presence mirroring); everyone else is
        // pinned to their authenticated id.
        let body_user = data.get("user_id").and_then(|v| v.as_str());
        let user_id = if ctx.auth_ctx.is_admin {
            body_user.or_else(|| ctx.auth_ctx.user_id.as_deref())
        } else {
            ctx.auth_ctx.user_id.as_deref()
        };
        let user_id = match user_id {
            Some(u) => u,
            None => {
                return (
                    401,
                    json_error("AUTH_REQUIRED", "authenticated session required"),
                )
            }
        };
        let user_data = data.get("data").cloned();

        let (snapshot, join_event) = match ctx.rooms.join(room, user_id, user_data) {
            Ok(result) => result,
            Err(e) => return (429, json_error(&e.code, &e.message)),
        };

        if let Ok(json) = serde_json::to_string(&join_event) {
            ctx.notifier.notify_presence(&json);
        }

        return (
            200,
            serde_json::json!({
                "joined": room,
                "snapshot": snapshot,
            })
            .to_string(),
        );
    }

    if url == "/api/rooms/leave" && method == HttpMethod::Post {
        if let Some(err) = require_auth(ctx) {
            return err;
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let room = match data.get("room").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return (400, json_error("MISSING_ROOM", "room is required")),
        };
        // Trust auth_ctx FIRST so a client can't impersonate another user by
        // putting someone else's id in the body. Admins may still override
        // (useful for server-to-server presence mirroring); everyone else is
        // pinned to their authenticated id.
        let body_user = data.get("user_id").and_then(|v| v.as_str());
        let user_id = if ctx.auth_ctx.is_admin {
            body_user.or_else(|| ctx.auth_ctx.user_id.as_deref())
        } else {
            ctx.auth_ctx.user_id.as_deref()
        };
        let user_id = match user_id {
            Some(u) => u,
            None => {
                return (
                    401,
                    json_error("AUTH_REQUIRED", "authenticated session required"),
                )
            }
        };

        if let Some(leave_event) = ctx.rooms.leave(room, user_id) {
            if let Ok(json) = serde_json::to_string(&leave_event) {
                ctx.notifier.notify_presence(&json);
            }
            return (200, serde_json::json!({"left": room}).to_string());
        }
        return (404, json_error("NOT_IN_ROOM", "User is not in this room"));
    }

    if url == "/api/rooms/presence" && method == HttpMethod::Post {
        if let Some(err) = require_auth(ctx) {
            return err;
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let room = match data.get("room").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return (400, json_error("MISSING_ROOM", "room is required")),
        };
        // Trust auth_ctx FIRST so a client can't impersonate another user by
        // putting someone else's id in the body. Admins may still override
        // (useful for server-to-server presence mirroring); everyone else is
        // pinned to their authenticated id.
        let body_user = data.get("user_id").and_then(|v| v.as_str());
        let user_id = if ctx.auth_ctx.is_admin {
            body_user.or_else(|| ctx.auth_ctx.user_id.as_deref())
        } else {
            ctx.auth_ctx.user_id.as_deref()
        };
        let user_id = match user_id {
            Some(u) => u,
            None => {
                return (
                    401,
                    json_error("AUTH_REQUIRED", "authenticated session required"),
                )
            }
        };
        let presence_data = data.get("data").cloned().unwrap_or(serde_json::json!({}));

        if let Some(presence_event) = ctx.rooms.set_presence(room, user_id, presence_data) {
            if let Ok(json) = serde_json::to_string(&presence_event) {
                ctx.notifier.notify_presence(&json);
            }
            return (200, serde_json::json!({"updated": true}).to_string());
        }
        return (404, json_error("NOT_IN_ROOM", "User is not in this room"));
    }

    if url == "/api/rooms/broadcast" && method == HttpMethod::Post {
        if let Some(err) = require_auth(ctx) {
            return err;
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let room = match data.get("room").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => return (400, json_error("MISSING_ROOM", "room is required")),
        };
        let topic = match data.get("topic").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return (400, json_error("MISSING_TOPIC", "topic is required")),
        };
        // Sender identity is server-resolved. Admin callers may spoof via
        // body (ops use cases); regular callers are fixed to their session.
        let body_sender = data.get("user_id").and_then(|v| v.as_str());
        let sender = if ctx.auth_ctx.is_admin {
            body_sender.or_else(|| ctx.auth_ctx.user_id.as_deref())
        } else {
            ctx.auth_ctx.user_id.as_deref()
        };
        let broadcast_data = data.get("data").cloned().unwrap_or(serde_json::json!({}));

        if let Some(broadcast_event) = ctx.rooms.broadcast(room, sender, topic, broadcast_data) {
            if let Ok(json) = serde_json::to_string(&broadcast_event) {
                ctx.notifier.notify_presence(&json);
            }
            return (200, serde_json::json!({"broadcasted": true}).to_string());
        }
        return (404, json_error("ROOM_NOT_FOUND", "Room does not exist"));
    }

    // GET /api/rooms
    if url == "/api/rooms" && method == HttpMethod::Get {
        if let Some(err) = require_auth(ctx) {
            return err;
        }
        let room_names = ctx.rooms.list_rooms();
        let rooms: Vec<serde_json::Value> = room_names
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "members": ctx.rooms.room_size(name),
                })
            })
            .collect();
        return (
            200,
            serde_json::to_string(&rooms).unwrap_or_else(|_| "[]".into()),
        );
    }

    // GET /api/rooms/:room
    if let Some(room_name) = url.strip_prefix("/api/rooms/") {
        let room_name = room_name.split('?').next().unwrap_or(room_name);
        if method == HttpMethod::Get
            && room_name != "join"
            && room_name != "leave"
            && room_name != "presence"
            && room_name != "broadcast"
        {
            if let Some(err) = require_auth(ctx) {
                return err;
            }
            let members = ctx.rooms.members(room_name);
            return (
                200,
                serde_json::json!({
                    "room": room_name,
                    "members": members,
                    "count": members.len(),
                })
                .to_string(),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Link / Unlink
    // -----------------------------------------------------------------------

    if url == "/api/link" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let entity = data.get("entity").and_then(|v| v.as_str()).unwrap_or("");
        let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let relation = data.get("relation").and_then(|v| v.as_str()).unwrap_or("");
        let target_id = data.get("target_id").and_then(|v| v.as_str()).unwrap_or("");

        // A link is a mutation: it sets a foreign key on the source row.
        // Apply the same write policy as PATCH /api/entities/:name/:id.
        let check = ctx
            .policy_engine
            .check_entity_write(entity, ctx.auth_ctx, Some(&data));
        if let pylon_policy::PolicyResult::Denied {
            policy_name,
            reason,
        } = check
        {
            tracing::warn!("[policy] link on {entity} denied by \"{policy_name}\": {reason}");
            return (403, json_error("POLICY_DENIED", "Access denied by policy"));
        }

        match ctx.store.link(entity, id, relation, target_id) {
            Ok(true) => return (200, serde_json::json!({"linked": true}).to_string()),
            Ok(false) => return (404, json_error("NOT_FOUND", "Source entity not found")),
            Err(e) => return (400, json_error(&e.code, &e.message)),
        }
    }

    if url == "/api/unlink" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let entity = data.get("entity").and_then(|v| v.as_str()).unwrap_or("");
        let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let relation = data.get("relation").and_then(|v| v.as_str()).unwrap_or("");

        let check = ctx
            .policy_engine
            .check_entity_write(entity, ctx.auth_ctx, Some(&data));
        if let pylon_policy::PolicyResult::Denied {
            policy_name,
            reason,
        } = check
        {
            tracing::warn!("[policy] unlink on {entity} denied by \"{policy_name}\": {reason}");
            return (403, json_error("POLICY_DENIED", "Access denied by policy"));
        }

        match ctx.store.unlink(entity, id, relation) {
            Ok(true) => return (200, serde_json::json!({"unlinked": true}).to_string()),
            Ok(false) => return (404, json_error("NOT_FOUND", "Source entity not found")),
            Err(e) => return (400, json_error(&e.code, &e.message)),
        }
    }

    // -----------------------------------------------------------------------
    // File upload/download
    // -----------------------------------------------------------------------

    if url == "/api/files/upload" && method == HttpMethod::Post {
        let (s, b) = ctx.files.upload(body);
        return (s, b);
    }

    if let Some(file_id) = url.strip_prefix("/api/files/") {
        let file_id = file_id.split('?').next().unwrap_or(file_id);
        if method == HttpMethod::Get {
            let (s, b) = ctx.files.get_file(file_id);
            return (s, b);
        }
    }

    // -----------------------------------------------------------------------
    // Transactions
    // -----------------------------------------------------------------------

    if url == "/api/transact" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return err;
        }
        let ops: Vec<serde_json::Value> = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };

        return match ctx.store.transact(&ops) {
            Ok((committed, results)) => (
                if committed { 200 } else { 400 },
                serde_json::json!({
                    "committed": committed,
                    "results": results,
                })
                .to_string(),
            ),
            Err(e) => (500, json_error(&e.code, &e.message)),
        };
    }

    // -----------------------------------------------------------------------
    // Query / Lookup / Graph
    // -----------------------------------------------------------------------

    // POST /api/query/:entity (filtered)
    if url.starts_with("/api/query/") && method == HttpMethod::Post {
        let entity = url
            .strip_prefix("/api/query/")
            .unwrap_or("")
            .split('?')
            .next()
            .unwrap_or("");
        if !entity.is_empty() && entity != "filtered" {
            let filter: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => {
                    return (
                        400,
                        json_error_safe(
                            "INVALID_JSON",
                            "Invalid request body",
                            &format!("Invalid JSON: {e}"),
                        ),
                    )
                }
            };
            match ctx.store.query_filtered(entity, &filter) {
                Ok(rows) => {
                    return (
                        200,
                        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()),
                    )
                }
                Err(e) => return (400, json_error(&e.code, &e.message)),
            }
        }
    }

    // GET /api/lookup/:entity/:field/:value
    if let Some(path) = url.strip_prefix("/api/lookup/") {
        let path = path.split('?').next().unwrap_or(path);
        let parts: Vec<&str> = path.splitn(3, '/').collect();
        if parts.len() == 3 && method == HttpMethod::Get {
            // Policy gate: a bare /api/lookup was a read-bypass that skipped
            // the entity policy check the main /api/entities/:name path has.
            // Same rule applies here.
            let check = ctx
                .policy_engine
                .check_entity_read(parts[0], ctx.auth_ctx, None);
            if let pylon_policy::PolicyResult::Denied {
                policy_name,
                reason,
            } = check
            {
                tracing::warn!(
                    "[policy] lookup on {} denied by \"{policy_name}\": {reason}",
                    parts[0]
                );
                return (403, json_error("POLICY_DENIED", "Access denied by policy"));
            }
            match ctx.store.lookup(parts[0], parts[1], parts[2]) {
                Ok(Some(row)) => {
                    return (
                        200,
                        serde_json::to_string(&row).unwrap_or_else(|_| "{}".into()),
                    )
                }
                Ok(None) => {
                    return (
                        404,
                        json_error(
                            "NOT_FOUND",
                            &format!("{}.{} = {} not found", parts[0], parts[1], parts[2]),
                        ),
                    )
                }
                Err(e) => return (400, json_error(&e.code, &e.message)),
            }
        }
    }

    // POST /api/aggregate/:entity — aggregation (count/sum/avg/min/max/groupBy)
    if let Some(rest) = url.strip_prefix("/api/aggregate/") {
        let entity = rest.split('?').next().unwrap_or(rest);
        if method == HttpMethod::Post && !entity.is_empty() {
            // Aggregations are reads in disguise — they expose row counts,
            // sums, and grouped-by distributions. Run the entity read
            // policy before dispatching.
            let check = ctx
                .policy_engine
                .check_entity_read(entity, ctx.auth_ctx, None);
            if let pylon_policy::PolicyResult::Denied {
                policy_name,
                reason,
            } = check
            {
                tracing::warn!(
                    "[policy] aggregate on {entity} denied by \"{policy_name}\": {reason}"
                );
                return (403, json_error("POLICY_DENIED", "Access denied by policy"));
            }
            let mut spec = match parse_json(body) {
                Ok(v) => v,
                Err((s, b)) => return (s, b),
            };
            // Tenant clamp — aggregates run raw SQL against the table and
            // would otherwise leak cross-tenant totals. If the caller has
            // an active tenant and the entity has an `orgId` column, force
            // the WHERE clause to include `orgId == auth.tenantId`. Any
            // client-supplied orgId is OVERWRITTEN by the server value, so
            // a malicious payload can't sum orders from another tenant.
            if let Some(tenant_id) = ctx.auth_ctx.tenant_id.as_deref() {
                let manifest = ctx.store.manifest();
                let has_org_id = manifest
                    .entities
                    .iter()
                    .find(|e| e.name == entity)
                    .map(|e| e.fields.iter().any(|f| f.name == "orgId"))
                    .unwrap_or(false);
                if has_org_id {
                    if let Some(obj) = spec.as_object_mut() {
                        let entry = obj
                            .entry("where".to_string())
                            .or_insert_with(|| serde_json::json!({}));
                        if let Some(where_obj) = entry.as_object_mut() {
                            where_obj.insert(
                                "orgId".to_string(),
                                serde_json::Value::String(tenant_id.to_string()),
                            );
                        }
                    }
                }
            }
            return match ctx.store.aggregate(entity, &spec) {
                Ok(result) => (
                    200,
                    serde_json::to_string(&result).unwrap_or_else(|_| "{}".into()),
                ),
                Err(e) => (400, json_error(&e.code, &e.message)),
            };
        }
    }

    // POST /api/query (graph)
    if url == "/api/query" && method == HttpMethod::Post {
        let query: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        // Gate every entity named in the graph against the read policy.
        // Without this a graph query could dump rows from entities the
        // caller is not allowed to see — the per-entity fence on
        // /api/entities was bypassed here.
        if let Some(obj) = query.as_object() {
            for entity_name in obj.keys() {
                let check = ctx
                    .policy_engine
                    .check_entity_read(entity_name, ctx.auth_ctx, None);
                if let pylon_policy::PolicyResult::Denied {
                    policy_name,
                    reason,
                } = check
                {
                    tracing::warn!(
                        "[policy] graph query on {entity_name} denied by \"{policy_name}\": {reason}"
                    );
                    return (403, json_error("POLICY_DENIED", "Access denied by policy"));
                }
            }
        }
        match ctx.store.query_graph(&query) {
            Ok(result) => {
                return (
                    200,
                    serde_json::to_string(&result).unwrap_or_else(|_| "{}".into()),
                )
            }
            Err(e) => return (400, json_error(&e.code, &e.message)),
        }
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    if let Some(action_name) = url.strip_prefix("/api/actions/") {
        let action_name = action_name.split('?').next().unwrap_or(action_name);
        if method != HttpMethod::Post {
            return (
                405,
                json_error("METHOD_NOT_ALLOWED", "Actions require POST"),
            );
        }

        let input: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };

        let policy_check = ctx
            .policy_engine
            .check_action(action_name, ctx.auth_ctx, Some(&input));
        if !policy_check.is_allowed() {
            if let pylon_policy::PolicyResult::Denied {
                policy_name,
                reason,
            } = policy_check
            {
                // Don't leak the raw allow expression to the client — it
                // reveals role names, field names, and the shape of the
                // access-control model. Log the full reason server-side
                // so operators can debug legitimate denials.
                tracing::warn!(
                    "[policy] action \"{action_name}\" denied by \"{policy_name}\": {reason}"
                );
                return (403, json_error("POLICY_DENIED", "Access denied by policy"));
            }
        }

        let manifest = ctx.store.manifest();
        let action_def = manifest.actions.iter().find(|a| a.name == action_name);
        if action_def.is_none() {
            let available: Vec<&str> = manifest.actions.iter().map(|a| a.name.as_str()).collect();
            return (
                404,
                json_error_with_hint(
                    "ACTION_NOT_FOUND",
                    &format!("Unknown action: \"{action_name}\""),
                    &format!("Available actions: [{}]", available.join(", ")),
                ),
            );
        }
        let action_def = action_def.unwrap();

        let input_obj = input.as_object();
        for field in &action_def.input {
            if !field.optional {
                let has_field = input_obj
                    .and_then(|o| o.get(&field.name))
                    .map(|v| !v.is_null())
                    .unwrap_or(false);
                if !has_field {
                    let required: Vec<String> = action_def
                        .input
                        .iter()
                        .filter(|f| !f.optional)
                        .map(|f| format!("{}: {}", f.name, f.field_type))
                        .collect();
                    return (
                        400,
                        json_error_with_hint(
                            "ACTION_MISSING_INPUT",
                            &format!(
                                "Required input field \"{}\" (type: {}) is missing for action \"{}\"",
                                field.name, field.field_type, action_name
                            ),
                            &format!("Required fields: [{}]", required.join(", ")),
                        ),
                    );
                }
            }
        }

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

    // -----------------------------------------------------------------------
    // Export
    // -----------------------------------------------------------------------

    if url == "/api/export" && method == HttpMethod::Get {
        if !ctx.auth_ctx.is_admin {
            return (
                403,
                json_error("FORBIDDEN", "Admin access required for data export"),
            );
        }
        let manifest = ctx.store.manifest();
        let mut entities_map = serde_json::Map::new();
        let mut counts_map = serde_json::Map::new();
        for ent in &manifest.entities {
            match ctx.store.list(&ent.name) {
                Ok(rows) => {
                    counts_map.insert(ent.name.clone(), serde_json::json!(rows.len()));
                    entities_map.insert(ent.name.clone(), serde_json::json!(rows));
                }
                Err(e) => {
                    return (
                        500,
                        json_error_safe(
                            "EXPORT_FAILED",
                            "Export operation failed",
                            &format!("Failed to export {}: {}", ent.name, e.message),
                        ),
                    );
                }
            }
        }
        let now = chrono_now_iso();
        return (
            200,
            serde_json::json!({
                "exported_at": now,
                "entities": entities_map,
                "counts": counts_map,
            })
            .to_string(),
        );
    }

    if let Some(entity_name) = url.strip_prefix("/api/export/") {
        let entity_name = entity_name.split('?').next().unwrap_or(entity_name);
        if method == HttpMethod::Get && !entity_name.is_empty() {
            if !ctx.auth_ctx.is_admin {
                return (
                    403,
                    json_error("FORBIDDEN", "Admin access required for data export"),
                );
            }
            match ctx.store.list(entity_name) {
                Ok(rows) => {
                    let now = chrono_now_iso();
                    let mut entities_map = serde_json::Map::new();
                    let mut counts_map = serde_json::Map::new();
                    counts_map.insert(entity_name.to_string(), serde_json::json!(rows.len()));
                    entities_map.insert(entity_name.to_string(), serde_json::json!(rows));
                    return (
                        200,
                        serde_json::json!({
                            "exported_at": now,
                            "entities": entities_map,
                            "counts": counts_map,
                        })
                        .to_string(),
                    );
                }
                Err(e) => return (400, json_error(&e.code, &e.message)),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Import — load a backup bundle (admin only)
    // -----------------------------------------------------------------------

    if url == "/api/import" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return err;
        }
        if !ctx.auth_ctx.is_admin {
            return (
                403,
                json_error("FORBIDDEN", "Admin access required for data import"),
            );
        }
        let data: serde_json::Value = match parse_json(body) {
            Ok(v) => v,
            Err((s, b)) => return (s, b),
        };
        // Bundle shape: { entities: { Name: [rows], ... }, cursor?: ... }
        let entities_obj = match data.get("entities").and_then(|v| v.as_object()) {
            Some(o) => o,
            None => {
                return (
                    400,
                    json_error("MISSING_FIELD", "Import requires `entities` object"),
                );
            }
        };

        let mut report: Vec<serde_json::Value> = Vec::new();
        let mut total_inserted: u64 = 0;
        let mut total_failed: u64 = 0;

        for (entity_name, rows_value) in entities_obj {
            let rows = match rows_value.as_array() {
                Some(a) => a,
                None => continue,
            };
            let mut inserted = 0u64;
            let mut failed = 0u64;
            for row in rows {
                let mut data = row.clone();
                // Strip id if you want auto-id, or preserve if present.
                // For import, we preserve to keep relations intact.
                if let Some(obj) = data.as_object_mut() {
                    obj.remove("__internal__"); // strip any internal fields
                }
                match ctx.store.insert(entity_name, &data) {
                    Ok(_) => inserted += 1,
                    Err(_) => failed += 1,
                }
            }
            total_inserted += inserted;
            total_failed += failed;
            report.push(serde_json::json!({
                "entity": entity_name,
                "inserted": inserted,
                "failed": failed,
            }));
        }

        return (
            200,
            serde_json::json!({
                "imported": total_inserted,
                "failed": total_failed,
                "by_entity": report,
            })
            .to_string(),
        );
    }

    // -----------------------------------------------------------------------
    // Cursor pagination
    // -----------------------------------------------------------------------

    if let Some(rest) = url.strip_prefix("/api/entities/") {
        let rest_no_qs = rest.split('?').next().unwrap_or(rest);
        if let Some(entity_name) = rest_no_qs.strip_suffix("/cursor") {
            if method == HttpMethod::Get {
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

                return match ctx.store.list_after(entity_name, after, limit + 1) {
                    Ok(rows) => {
                        let has_more = rows.len() > limit;
                        let page: Vec<serde_json::Value> = rows.into_iter().take(limit).collect();
                        let next_cursor = page
                            .last()
                            .and_then(|r| r.get("id"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        (
                            200,
                            serde_json::json!({
                                "data": page,
                                "next_cursor": next_cursor,
                                "has_more": has_more,
                            })
                            .to_string(),
                        )
                    }
                    Err(e) => (400, json_error(&e.code, &e.message)),
                };
            }
        }
    }

    // -----------------------------------------------------------------------
    // Batch operations
    // -----------------------------------------------------------------------

    if url == "/api/batch" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return err;
        }
        let batch: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let ops = match batch.get("operations").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => {
                return (
                    400,
                    json_error(
                        "MISSING_OPERATIONS",
                        "Request body must contain an \"operations\" array",
                    ),
                )
            }
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
                    match ctx.store.insert(entity, &data) {
                        Ok(id) => {
                            let seq = ctx.change_log.append(
                                entity,
                                &id,
                                ChangeKind::Insert,
                                Some(data.clone()),
                            );
                            broadcast_change(
                                ctx.notifier,
                                seq,
                                entity,
                                &id,
                                ChangeKind::Insert,
                                Some(&data),
                            );
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
                    match ctx.store.update(entity, id, &data) {
                        Ok(updated) => {
                            if updated {
                                let seq = ctx.change_log.append(
                                    entity,
                                    id,
                                    ChangeKind::Update,
                                    Some(data.clone()),
                                );
                                broadcast_change(
                                    ctx.notifier,
                                    seq,
                                    entity,
                                    id,
                                    ChangeKind::Update,
                                    Some(&data),
                                );
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
                    match ctx.store.delete(entity, id) {
                        Ok(deleted) => {
                            if deleted {
                                let seq =
                                    ctx.change_log.append(entity, id, ChangeKind::Delete, None);
                                broadcast_change(
                                    ctx.notifier,
                                    seq,
                                    entity,
                                    id,
                                    ChangeKind::Delete,
                                    None,
                                );
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

        return (
            200,
            serde_json::json!({
                "results": results,
                "succeeded": succeeded,
                "failed": failed,
            })
            .to_string(),
        );
    }

    // -----------------------------------------------------------------------
    // Entity CRUD
    // -----------------------------------------------------------------------

    if let Some(path) = url.strip_prefix("/api/entities/") {
        let path = path.split('?').next().unwrap_or(path);
        let segments: Vec<&str> = path.splitn(2, '/').collect();
        let entity_name = segments[0];
        let entity_id = segments.get(1).filter(|s| !s.is_empty()).copied();

        // Policy enforcement — check BEFORE dispatching. Previously only GET
        // was gated, so POST/PATCH/DELETE silently bypassed entity policies
        // entirely (a broad authz bypass).
        //
        // For writes we parse the body first so the policy sees the incoming
        // data (field-level rules can depend on it). Parse errors short-circuit
        // to 400 before we touch the store.
        let parsed_body_for_policy: Option<serde_json::Value> = match method {
            HttpMethod::Post | HttpMethod::Patch if !body.trim().is_empty() => {
                match serde_json::from_str(body) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        return (
                            400,
                            json_error_safe(
                                "INVALID_JSON",
                                "Invalid request body",
                                &format!("Invalid JSON: {e}"),
                            ),
                        );
                    }
                }
            }
            _ => None,
        };

        // For PATCH and DELETE we want ownership rules like
        // `data.authorId == auth.userId` to evaluate against the EXISTING
        // row, not the incoming patch — otherwise a caller could sidestep
        // the gate by omitting the ownership field from their PATCH body.
        // We fetch the row once here and reuse it for the policy call.
        let existing_row_for_policy: Option<serde_json::Value> = match (method, entity_id) {
            (HttpMethod::Patch, Some(id)) | (HttpMethod::Delete, Some(id)) => {
                ctx.store.get_by_id(entity_name, id).ok().flatten()
            }
            _ => None,
        };

        let policy_check = match method {
            HttpMethod::Get => ctx
                .policy_engine
                .check_entity_read(entity_name, ctx.auth_ctx, None),
            HttpMethod::Post => ctx.policy_engine.check_entity_insert(
                entity_name,
                ctx.auth_ctx,
                parsed_body_for_policy.as_ref(),
            ),
            HttpMethod::Patch => ctx.policy_engine.check_entity_update(
                entity_name,
                ctx.auth_ctx,
                existing_row_for_policy.as_ref(),
            ),
            HttpMethod::Delete => ctx.policy_engine.check_entity_delete(
                entity_name,
                ctx.auth_ctx,
                existing_row_for_policy.as_ref(),
            ),
            _ => pylon_policy::PolicyResult::Allowed,
        };
        if let pylon_policy::PolicyResult::Denied {
            policy_name,
            reason,
        } = policy_check
        {
            tracing::warn!(
                "[policy] {method:?} {entity_name} denied by \"{policy_name}\": {reason}"
            );
            return (
                403,
                json_error_with_hint(
                    "POLICY_DENIED",
                    "Access denied by policy",
                    "Check your auth token or the policy rules in your schema",
                ),
            );
        }

        return match (method, entity_id) {
            (HttpMethod::Get, None) => handle_list(ctx.store, entity_name, url),
            (HttpMethod::Post, None) => handle_insert(ctx, entity_name, body),
            (HttpMethod::Get, Some(id)) => handle_get(ctx.store, entity_name, id),
            (HttpMethod::Patch, Some(id)) => handle_update(ctx, entity_name, id, body),
            (HttpMethod::Delete, Some(id)) => handle_delete(ctx, entity_name, id),
            _ => (405, json_error("METHOD_NOT_ALLOWED", "Method not allowed")),
        };
    }

    // -----------------------------------------------------------------------
    // Cache API
    // -----------------------------------------------------------------------

    if url == "/api/cache" && method == HttpMethod::Post {
        return ctx.cache.handle_command(body);
    }

    if let Some(cache_key) = url.strip_prefix("/api/cache/") {
        let cache_key = cache_key.split('?').next().unwrap_or(cache_key);
        if method == HttpMethod::Get && !cache_key.is_empty() {
            return ctx.cache.handle_get(cache_key);
        }
        if method == HttpMethod::Delete && !cache_key.is_empty() {
            return ctx.cache.handle_delete(cache_key);
        }
    }

    // -----------------------------------------------------------------------
    // Pub/Sub API
    // -----------------------------------------------------------------------

    if url == "/api/pubsub/publish" && method == HttpMethod::Post {
        return ctx.pubsub.handle_publish(body);
    }

    if url == "/api/pubsub/channels" && method == HttpMethod::Get {
        return ctx.pubsub.handle_channels();
    }

    if let Some(channel_name) = url.strip_prefix("/api/pubsub/history/") {
        let channel_name = channel_name.split('?').next().unwrap_or(channel_name);
        if method == HttpMethod::Get && !channel_name.is_empty() {
            return ctx.pubsub.handle_history(channel_name, url);
        }
    }

    // -----------------------------------------------------------------------
    // Jobs API
    // -----------------------------------------------------------------------

    if url == "/api/jobs" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return err;
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let name = match data.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return (400, json_error("MISSING_NAME", "name is required")),
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
        return (
            201,
            serde_json::json!({"id": id, "status": "pending"}).to_string(),
        );
    }

    if url == "/api/jobs/stats" && method == HttpMethod::Get {
        let stats = ctx.jobs.stats();
        return (
            200,
            serde_json::to_string(&stats).unwrap_or_else(|_| "{}".into()),
        );
    }

    if url == "/api/jobs/dead" && method == HttpMethod::Get {
        let dead = ctx.jobs.dead_letters();
        return (
            200,
            serde_json::to_string(&dead).unwrap_or_else(|_| "[]".into()),
        );
    }

    if let Some(rest) = url.strip_prefix("/api/jobs/dead/") {
        let rest = rest.split('?').next().unwrap_or(rest);
        if let Some(job_id) = rest.strip_suffix("/retry") {
            if method == HttpMethod::Post && !job_id.is_empty() {
                if let Some(err) = require_admin(ctx) {
                    return err;
                }
                if ctx.jobs.retry_dead(job_id) {
                    return (
                        200,
                        serde_json::json!({"retried": true, "id": job_id}).to_string(),
                    );
                }
                return (
                    404,
                    json_error("NOT_FOUND", "Job not found in dead letter queue"),
                );
            }
        }
    }

    if url.starts_with("/api/jobs") && method == HttpMethod::Get {
        let path = url.split('?').next().unwrap_or(url);
        if path == "/api/jobs" {
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
            return (
                200,
                serde_json::to_string(&jobs).unwrap_or_else(|_| "[]".into()),
            );
        }
    }

    if let Some(job_id) = url.strip_prefix("/api/jobs/") {
        let job_id = job_id.split('?').next().unwrap_or(job_id);
        if method == HttpMethod::Get && !job_id.is_empty() && job_id != "stats" && job_id != "dead"
        {
            if let Some(job) = ctx.jobs.get_job(job_id) {
                return (
                    200,
                    serde_json::to_string(&job).unwrap_or_else(|_| "{}".into()),
                );
            }
            return (
                404,
                json_error("NOT_FOUND", &format!("Job {job_id} not found")),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Scheduler API
    // -----------------------------------------------------------------------

    if url == "/api/scheduler" && method == HttpMethod::Get {
        let tasks = ctx.scheduler.list_tasks();
        return (
            200,
            serde_json::to_string(&tasks).unwrap_or_else(|_| "[]".into()),
        );
    }

    if let Some(task_name) = url.strip_prefix("/api/scheduler/trigger/") {
        let task_name = task_name.split('?').next().unwrap_or(task_name);
        if method == HttpMethod::Post && !task_name.is_empty() {
            if let Some(err) = require_admin(ctx) {
                return err;
            }
            if ctx.scheduler.trigger(task_name) {
                return (
                    200,
                    serde_json::json!({"triggered": true, "task": task_name}).to_string(),
                );
            }
            return (
                404,
                json_error(
                    "NOT_FOUND",
                    &format!("Scheduled task \"{task_name}\" not found"),
                ),
            );
        }
    }

    // -----------------------------------------------------------------------
    // TypeScript Functions API
    // -----------------------------------------------------------------------

    // GET /api/fn — list registered functions
    if url == "/api/fn" && method == HttpMethod::Get {
        return match ctx.functions {
            Some(f) => (
                200,
                serde_json::to_string(&f.list_fns()).unwrap_or_else(|_| "[]".into()),
            ),
            None => (200, "[]".into()),
        };
    }

    // GET /api/fn/traces — recent function traces (observability)
    if url.starts_with("/api/fn/traces") && method == HttpMethod::Get {
        return match ctx.functions {
            Some(f) => {
                let limit: usize = query_param(url, "limit")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(50)
                    .min(500);
                let traces = f.recent_traces(limit);
                (
                    200,
                    serde_json::to_string(&traces).unwrap_or_else(|_| "[]".into()),
                )
            }
            None => (200, "[]".into()),
        };
    }

    // /api/webhooks/:action_name — invoke an action with full HTTP request
    // context available via `ctx.request`. Use this instead of /api/fn/...
    // when you need raw headers + body bytes (Stripe webhooks, GitHub
    // webhooks, Slack events — anywhere the caller signs the payload).
    //
    // Any HTTP verb is accepted so providers that use GET for some event
    // types (e.g. challenge responses) work out of the box.
    if let Some(action_name) = url.strip_prefix("/api/webhooks/") {
        let action_name = action_name.split('?').next().unwrap_or(action_name);
        if !action_name.is_empty() {
            let fn_ops = match ctx.functions {
                Some(f) => f,
                None => {
                    return (
                        503,
                        json_error(
                            "FUNCTIONS_NOT_AVAILABLE",
                            "TypeScript function runtime is not configured",
                        ),
                    );
                }
            };
            let def = match fn_ops.get_fn(action_name) {
                Some(d) => d,
                None => {
                    return (
                        404,
                        json_error(
                            "FN_NOT_FOUND",
                            &format!("Action \"{action_name}\" is not registered"),
                        ),
                    );
                }
            };
            // Only actions can be webhook targets. Mutations run under a
            // write transaction and can't safely handle arbitrary external
            // input; queries are read-only. The action type exists for
            // "external I/O, non-transactional" exactly like this case.
            if def.fn_type != pylon_functions::protocol::FnType::Action {
                return (
                    400,
                    json_error(
                        "NOT_AN_ACTION",
                        &format!("\"{action_name}\" is not an action — only actions can be webhook targets"),
                    ),
                );
            }

            let auth = pylon_functions::protocol::AuthInfo {
                user_id: ctx.auth_ctx.user_id.clone(),
                is_admin: ctx.auth_ctx.is_admin,
                tenant_id: ctx.auth_ctx.tenant_id.clone(),
            };

            // Rate-limit on the action name so a bad signature check can't
            // be used to amplify load on our function runtime.
            let identity = auth.user_id.as_deref().unwrap_or("anon");
            if let Err(retry_after) = fn_ops.check_rate_limit(action_name, identity) {
                let body = format!(
                    r#"{{"error":{{"code":"RATE_LIMITED","message":"Webhook \"{action_name}\" rate limit exceeded","retry_after_secs":{retry_after}}}}}"#
                );
                return (429, body);
            }

            // Build RequestInfo — this is the whole point. Pass raw method,
            // path, headers (already lowercased by the transport layer or
            // we lowercase here for consistency), and body bytes exactly
            // as received so signature checks work.
            let mut headers = std::collections::HashMap::new();
            for (name, value) in ctx.request_headers {
                headers
                    .entry(name.to_ascii_lowercase())
                    .and_modify(|existing: &mut String| {
                        existing.push_str(", ");
                        existing.push_str(value);
                    })
                    .or_insert_with(|| value.clone());
            }
            let request = pylon_functions::protocol::RequestInfo {
                method: format!("{:?}", method).to_uppercase(),
                path: url.to_string(),
                headers,
                raw_body: body.to_string(),
            };

            // Actions get the whole body as a string input argument too
            // — some handlers will re-parse it themselves, others just
            // care about ctx.request.rawBody. Passing both is cheap.
            let args = serde_json::json!({ "rawBody": body });

            return match fn_ops.call(action_name, args, auth, None, Some(request)) {
                Ok((value, _trace)) => (
                    200,
                    serde_json::to_string(&value).unwrap_or_else(|_| "null".into()),
                ),
                Err(e) => (400, json_error(&e.code, &e.message)),
            };
        }
    }

    // POST /api/fn/:name — invoke a function
    if let Some(fn_name) = url.strip_prefix("/api/fn/") {
        let fn_name = fn_name.split('?').next().unwrap_or(fn_name);
        if method == HttpMethod::Post && !fn_name.is_empty() && fn_name != "traces" {
            let fn_ops = match ctx.functions {
                Some(f) => f,
                None => {
                    return (
                        503,
                        json_error(
                            "FUNCTIONS_NOT_AVAILABLE",
                            "TypeScript function runtime is not configured",
                        ),
                    );
                }
            };

            // Look up the function to know its type (for fn existence check).
            if fn_ops.get_fn(fn_name).is_none() {
                return (
                    404,
                    json_error(
                        "FN_NOT_FOUND",
                        &format!("Function \"{fn_name}\" is not registered"),
                    ),
                );
            }

            // Parse args.
            let args: serde_json::Value = if body.trim().is_empty() {
                serde_json::json!({})
            } else {
                match parse_json(body) {
                    Ok(v) => v,
                    Err((s, b)) => return (s, b),
                }
            };

            let auth = pylon_functions::protocol::AuthInfo {
                user_id: ctx.auth_ctx.user_id.clone(),
                is_admin: ctx.auth_ctx.is_admin,
                tenant_id: ctx.auth_ctx.tenant_id.clone(),
            };

            // Per-function rate limit. Identity is the user id when
            // authenticated, falling back to "anon" so unauth abuse is
            // bounded as a single bucket.
            let identity = auth.user_id.as_deref().unwrap_or("anon");
            if let Err(retry_after) = fn_ops.check_rate_limit(fn_name, identity) {
                let body = format!(
                    r#"{{"error":{{"code":"RATE_LIMITED","message":"Function \"{fn_name}\" rate limit exceeded","retry_after_secs":{retry_after}}}}}"#
                );
                return (429, body);
            }

            // For non-streaming responses we don't pass a callback. The server
            // layer handles streaming by passing its own callback through.
            // `request: None` here — this is the /api/fn/... path where the
            // client has already parsed args from the body, so raw request
            // metadata isn't meaningful. The HTTP-route binding path (below)
            // does pass `request` for webhook use cases.
            return match fn_ops.call(fn_name, args, auth, None, None) {
                Ok((value, _trace)) => (
                    200,
                    serde_json::to_string(&value).unwrap_or_else(|_| "null".into()),
                ),
                Err(e) => (400, json_error(&e.code, &e.message)),
            };
        }
    }

    // -----------------------------------------------------------------------
    // Shards (real-time simulations: games, MMO zones, live docs, etc.)
    // -----------------------------------------------------------------------

    if url == "/api/shards" && method == HttpMethod::Get {
        return match ctx.shards {
            Some(s) => {
                let ids = s.list_shards();
                let out: Vec<serde_json::Value> = ids
                    .iter()
                    .map(|id| {
                        let info = s
                            .get_shard(id)
                            .map(|sh| {
                                serde_json::json!({
                                    "id": sh.id(),
                                    "running": sh.is_running(),
                                    "tick": sh.tick_number(),
                                    "subscribers": sh.subscriber_count(),
                                    "input_queue": sh.input_queue_len(),
                                })
                            })
                            .unwrap_or(serde_json::json!({"id": id}));
                        info
                    })
                    .collect();
                (
                    200,
                    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into()),
                )
            }
            None => (200, "[]".into()),
        };
    }

    // POST /api/shards/:id/input — send an input to a shard
    if method == HttpMethod::Post {
        if let Some(rest) = url.strip_prefix("/api/shards/") {
            let rest = rest.split('?').next().unwrap_or(rest);
            if let Some(shard_id) = rest.strip_suffix("/input") {
                let shards = match ctx.shards {
                    Some(s) => s,
                    None => {
                        return (
                            503,
                            json_error("SHARDS_NOT_AVAILABLE", "Shard system is not configured"),
                        );
                    }
                };
                let shard = match shards.get_shard(shard_id) {
                    Some(s) => s,
                    None => {
                        return (
                            404,
                            json_error(
                                "SHARD_NOT_FOUND",
                                &format!("Shard \"{shard_id}\" not found"),
                            ),
                        );
                    }
                };

                // Parse envelope: { subscriber_id?, client_seq?, input }
                let envelope: serde_json::Value = match parse_json(body) {
                    Ok(v) => v,
                    Err((s, b)) => return (s, b),
                };
                let subscriber_id = envelope
                    .get("subscriber_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| ctx.auth_ctx.user_id.clone())
                    .unwrap_or_else(|| format!("anon_{}", query_param(url, "sid").unwrap_or("0")));
                let client_seq = envelope.get("client_seq").and_then(|v| v.as_u64());
                let input = envelope
                    .get("input")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let input_str = serde_json::to_string(&input).unwrap_or_else(|_| "null".into());

                let shard_auth = pylon_realtime::ShardAuth {
                    user_id: ctx.auth_ctx.user_id.clone(),
                    is_admin: ctx.auth_ctx.is_admin,
                };
                return match shard.push_input_json(
                    pylon_realtime::SubscriberId::new(subscriber_id),
                    &input_str,
                    client_seq,
                    &shard_auth,
                ) {
                    Ok(seq) => (
                        200,
                        serde_json::json!({"accepted": true, "seq": seq}).to_string(),
                    ),
                    Err(pylon_realtime::ShardError::Unauthorized(reason)) => {
                        (403, json_error("UNAUTHORIZED", &reason))
                    }
                    Err(e) => (400, json_error("INPUT_REJECTED", &e.to_string())),
                };
            }
        }
    }

    // GET /api/shards/:id — shard info
    if method == HttpMethod::Get {
        if let Some(shard_id) = url.strip_prefix("/api/shards/") {
            let shard_id = shard_id.split('?').next().unwrap_or(shard_id);
            // Skip the /connect subpath — that's server-level SSE.
            if !shard_id.is_empty() && !shard_id.contains('/') {
                if let Some(shards) = ctx.shards {
                    if let Some(sh) = shards.get_shard(shard_id) {
                        return (
                            200,
                            serde_json::json!({
                                "id": sh.id(),
                                "running": sh.is_running(),
                                "tick": sh.tick_number(),
                                "subscribers": sh.subscriber_count(),
                                "input_queue": sh.input_queue_len(),
                            })
                            .to_string(),
                        );
                    }
                    return (
                        404,
                        json_error(
                            "SHARD_NOT_FOUND",
                            &format!("Shard \"{shard_id}\" not found"),
                        ),
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Workflow Engine API
    // -----------------------------------------------------------------------

    if url == "/api/workflows/definitions" && method == HttpMethod::Get {
        let defs = ctx.workflows.definitions();
        return (
            200,
            serde_json::to_string(&defs).unwrap_or_else(|_| "[]".into()),
        );
    }

    if url == "/api/workflows/start" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return err;
        }
        let data: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return (
                    400,
                    json_error_safe(
                        "INVALID_JSON",
                        "Invalid request body",
                        &format!("Invalid JSON: {e}"),
                    ),
                )
            }
        };
        let name = match data.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_string(),
            None => return (400, json_error("MISSING_FIELD", "\"name\" is required")),
        };
        let input = data.get("input").cloned().unwrap_or(serde_json::json!({}));
        match ctx.workflows.start(&name, input) {
            Ok(id) => return (201, serde_json::json!({"id": id}).to_string()),
            Err(e) => return (400, json_error("WORKFLOW_START_FAILED", &e)),
        }
    }

    if url.starts_with("/api/workflows")
        && !url.starts_with("/api/workflows/")
        && method == HttpMethod::Get
    {
        let status_filter = url
            .split("status=")
            .nth(1)
            .and_then(|s| s.split('&').next());
        let instances = ctx.workflows.list(status_filter);
        return (
            200,
            serde_json::to_string(&instances).unwrap_or_else(|_| "[]".into()),
        );
    }

    if let Some(rest) = url.strip_prefix("/api/workflows/") {
        let rest = rest.split('?').next().unwrap_or(rest);
        let (wf_id, sub) = match rest.find('/') {
            Some(i) => (&rest[..i], Some(&rest[i + 1..])),
            None => (rest, None),
        };

        if !wf_id.is_empty() && !wf_id.starts_with("definitions") {
            match (method, sub) {
                (HttpMethod::Get, None) => {
                    return match ctx.workflows.get(wf_id) {
                        Some(inst) => (
                            200,
                            serde_json::to_string(&inst).unwrap_or_else(|_| "{}".into()),
                        ),
                        None => (
                            404,
                            json_error("NOT_FOUND", &format!("Workflow {wf_id} not found")),
                        ),
                    };
                }
                (HttpMethod::Post, Some("advance")) => {
                    if let Some(err) = require_admin(ctx) {
                        return err;
                    }
                    return match ctx.workflows.advance(wf_id) {
                        Ok(status) => (200, serde_json::json!({"status": status}).to_string()),
                        Err(e) => (400, json_error("WORKFLOW_ADVANCE_FAILED", &e)),
                    };
                }
                (HttpMethod::Post, Some("event")) => {
                    if let Some(err) = require_admin(ctx) {
                        return err;
                    }
                    let data: serde_json::Value = match serde_json::from_str(body) {
                        Ok(v) => v,
                        Err(e) => {
                            return (
                                400,
                                json_error_safe(
                                    "INVALID_JSON",
                                    "Invalid request body",
                                    &format!("Invalid JSON: {e}"),
                                ),
                            )
                        }
                    };
                    let event = match data.get("event").and_then(|v| v.as_str()) {
                        Some(e) => e.to_string(),
                        None => return (400, json_error("MISSING_FIELD", "\"event\" is required")),
                    };
                    let event_data = data.get("data").cloned().unwrap_or(serde_json::json!({}));
                    return match ctx.workflows.send_event(wf_id, &event, event_data) {
                        Ok(()) => (200, serde_json::json!({"ok": true}).to_string()),
                        Err(e) => (400, json_error("WORKFLOW_EVENT_FAILED", &e)),
                    };
                }
                (HttpMethod::Post, Some("cancel")) => {
                    if let Some(err) = require_admin(ctx) {
                        return err;
                    }
                    return match ctx.workflows.cancel(wf_id) {
                        Ok(()) => (200, serde_json::json!({"cancelled": true}).to_string()),
                        Err(e) => (400, json_error("WORKFLOW_CANCEL_FAILED", &e)),
                    };
                }
                _ => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // AI completion (non-streaming) — only available on platforms with env vars
    // -----------------------------------------------------------------------

    if url == "/api/ai/complete" && method == HttpMethod::Post {
        // AI completion requires env var configuration. On Workers, these
        // come from wrangler secrets. We return 503 here; the server-level
        // handler owns the streaming variant since it needs I/O.
        return (
            503,
            json_error(
                "AI_NOT_CONFIGURED",
                "AI completion is not available on this platform",
            ),
        );
    }

    // -----------------------------------------------------------------------
    // Fallback
    // -----------------------------------------------------------------------

    (
        404,
        json_error_with_hint(
            "NOT_FOUND",
            &format!("No API route matches {url}"),
            "Available endpoints: /api/entities/<entity>, /api/actions/<name>, /api/query, /api/auth/*, /api/sync/*, /api/files/*, /api/cache, /api/pubsub/*, /api/jobs, /api/scheduler, /api/workflows, /api/ai/*, /studio",
        ),
    )
}

// ---------------------------------------------------------------------------
// Entity CRUD helpers
// ---------------------------------------------------------------------------

fn handle_list(store: &dyn DataStore, entity: &str, url: &str) -> (u16, String) {
    let limit: Option<usize> = url
        .split("limit=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .and_then(|s| s.parse().ok());
    let offset: usize = url
        .split("offset=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Push limit/offset down to SQL via query_filtered's $limit/$offset.
    // Skips full-table scans for large entities.
    let mut filter = serde_json::Map::new();
    if let Some(l) = limit {
        filter.insert("$limit".into(), serde_json::json!(l));
    }
    if offset > 0 {
        filter.insert("$offset".into(), serde_json::json!(offset));
    }
    let filter = serde_json::Value::Object(filter);

    match store.query_filtered(entity, &filter) {
        Ok(rows) => {
            // For backwards compatibility we keep the response shape but
            // `total` here means "rows returned" not "total in table".
            // The cursor pagination endpoint at /api/entities/:e/cursor is
            // the right path for total counts at scale.
            let count = rows.len();
            (
                200,
                serde_json::json!({
                    "data": rows,
                    "count": count,
                    "offset": offset,
                    "limit": limit,
                })
                .to_string(),
            )
        }
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

fn handle_get(store: &dyn DataStore, entity: &str, id: &str) -> (u16, String) {
    match store.get_by_id(entity, id) {
        Ok(Some(row)) => (
            200,
            serde_json::to_string(&row).unwrap_or_else(|_| "{}".into()),
        ),
        Ok(None) => (
            404,
            json_error("NOT_FOUND", &format!("{entity} with id \"{id}\" not found")),
        ),
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

fn handle_insert(ctx: &RouterContext, entity: &str, body: &str) -> (u16, String) {
    let mut data: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                json_error_safe(
                    "INVALID_JSON",
                    "Invalid request body",
                    &format!("Invalid JSON: {e}"),
                ),
            )
        }
    };
    // Run plugin `before_insert` hooks. Registered plugins (validation,
    // timestamps, slugify) mutate `data` here; a rejected hook aborts
    // the write with their status + error payload.
    if let Err((status, code, msg)) =
        ctx.plugin_hooks
            .before_insert(entity, &mut data, ctx.auth_ctx)
    {
        return (status, json_error(&code, &msg));
    }
    match ctx.store.insert(entity, &data) {
        Ok(id) => {
            let seq = ctx
                .change_log
                .append(entity, &id, ChangeKind::Insert, Some(data.clone()));
            broadcast_change(
                ctx.notifier,
                seq,
                entity,
                &id,
                ChangeKind::Insert,
                Some(&data),
            );
            ctx.plugin_hooks
                .after_insert(entity, &id, &data, ctx.auth_ctx);
            (201, serde_json::json!({"id": id}).to_string())
        }
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

fn handle_update(ctx: &RouterContext, entity: &str, id: &str, body: &str) -> (u16, String) {
    let mut data: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                json_error_safe(
                    "INVALID_JSON",
                    "Invalid request body",
                    &format!("Invalid JSON: {e}"),
                ),
            )
        }
    };
    if let Err((status, code, msg)) =
        ctx.plugin_hooks
            .before_update(entity, id, &mut data, ctx.auth_ctx)
    {
        return (status, json_error(&code, &msg));
    }
    match ctx.store.update(entity, id, &data) {
        Ok(true) => {
            let seq = ctx
                .change_log
                .append(entity, id, ChangeKind::Update, Some(data.clone()));
            broadcast_change(
                ctx.notifier,
                seq,
                entity,
                id,
                ChangeKind::Update,
                Some(&data),
            );
            ctx.plugin_hooks
                .after_update(entity, id, &data, ctx.auth_ctx);
            (200, serde_json::json!({"updated": true}).to_string())
        }
        Ok(false) => (
            404,
            json_error("NOT_FOUND", &format!("{entity}/{id} not found")),
        ),
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

fn handle_delete(ctx: &RouterContext, entity: &str, id: &str) -> (u16, String) {
    if let Err((status, code, msg)) = ctx.plugin_hooks.before_delete(entity, id, ctx.auth_ctx) {
        return (status, json_error(&code, &msg));
    }
    match ctx.store.delete(entity, id) {
        Ok(true) => {
            let seq = ctx.change_log.append(entity, id, ChangeKind::Delete, None);
            broadcast_change(ctx.notifier, seq, entity, id, ChangeKind::Delete, None);
            ctx.plugin_hooks.after_delete(entity, id, ctx.auth_ctx);
            (200, serde_json::json!({"deleted": true}).to_string())
        }
        Ok(false) => (
            404,
            json_error("NOT_FOUND", &format!("{entity}/{id} not found")),
        ),
        Err(e) => (400, json_error(&e.code, &e.message)),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn broadcast_change(
    notifier: &dyn ChangeNotifier,
    seq: u64,
    entity: &str,
    row_id: &str,
    kind: ChangeKind,
    data: Option<&serde_json::Value>,
) {
    let event = pylon_sync::ChangeEvent {
        seq,
        entity: entity.to_string(),
        row_id: row_id.to_string(),
        kind,
        data: data.cloned(),
        timestamp: String::new(),
    };
    notifier.notify(&event);
}

pub fn json_error(code: &str, message: &str) -> String {
    serde_json::json!({"error": {"code": code, "message": message}}).to_string()
}

/// Conventional field names that link a row to a user. Used by the GDPR
/// export/purge endpoints to discover referencing rows without requiring
/// app authors to annotate their schema. Apps that store user references
/// under non-conventional names must supply their own purge hook — the
/// default won't find them, and GDPR-compliant deletes require a full
/// sweep. We log a warning on export when entities appear to reference
/// users through custom names not in this list.
const USER_REF_FIELDS: &[&str] = &[
    "userId",
    "user_id",
    "authorId",
    "author_id",
    "ownerId",
    "owner_id",
    "createdBy",
    "created_by",
];

/// Build a data-subject export for `user_id`. Returns a JSON envelope with
/// every row referencing the user across every manifest entity, plus the
/// User row itself when present. Format is stable — clients can diff
/// exports across time to see what's changed.
fn gdpr_export(ctx: &RouterContext, user_id: &str) -> (u16, String) {
    let manifest = ctx.store.manifest();
    let mut entities = serde_json::Map::new();

    // The User row itself (if the schema has a User entity).
    if let Ok(Some(user_row)) = ctx.store.get_by_id("User", user_id) {
        entities.insert("User".to_string(), serde_json::json!([user_row]));
    }

    // Every entity that has a user-ref field: list all rows matching.
    for ent in &manifest.entities {
        if ent.name == "User" {
            continue; // Already captured above.
        }
        let user_field = ent
            .fields
            .iter()
            .find(|f| USER_REF_FIELDS.contains(&f.name.as_str()));
        let Some(field) = user_field else { continue };
        let filter = serde_json::json!({ &field.name: user_id });
        match ctx.store.query_filtered(&ent.name, &filter) {
            Ok(rows) if !rows.is_empty() => {
                entities.insert(ent.name.clone(), serde_json::Value::Array(rows));
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("[gdpr] export: query {} failed: {}", ent.name, e.message);
            }
        }
    }

    let envelope = serde_json::json!({
        "user_id": user_id,
        "exported_at": pylon_kernel::util::now_iso(),
        "entities": entities,
    });
    (200, envelope.to_string())
}

/// Hard-delete every row tied to `user_id` and revoke all active sessions.
/// Best-effort: a per-entity error is logged and counted but does not abort
/// the sweep — partial success is better than leaving half the footprint.
fn gdpr_purge(ctx: &RouterContext, user_id: &str) -> (u16, String) {
    let manifest = ctx.store.manifest();
    let mut deleted: u64 = 0;
    let mut errors: Vec<String> = Vec::new();

    // Delete the User row itself first.
    if let Ok(true) = ctx.store.delete("User", user_id) {
        deleted += 1;
        // Synthetic change event so sync clients notice.
        let seq = ctx
            .change_log
            .append("User", user_id, ChangeKind::Delete, None);
        broadcast_change(ctx.notifier, seq, "User", user_id, ChangeKind::Delete, None);
    }

    // Referencing rows.
    for ent in &manifest.entities {
        if ent.name == "User" {
            continue;
        }
        let Some(field) = ent
            .fields
            .iter()
            .find(|f| USER_REF_FIELDS.contains(&f.name.as_str()))
        else {
            continue;
        };
        let filter = serde_json::json!({ &field.name: user_id });
        let rows = match ctx.store.query_filtered(&ent.name, &filter) {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("query {}: {}", ent.name, e.message));
                continue;
            }
        };
        for row in rows {
            let Some(id) = row.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            match ctx.store.delete(&ent.name, id) {
                Ok(true) => {
                    deleted += 1;
                    let seq = ctx
                        .change_log
                        .append(&ent.name, id, ChangeKind::Delete, None);
                    broadcast_change(ctx.notifier, seq, &ent.name, id, ChangeKind::Delete, None);
                }
                Ok(false) => {}
                Err(e) => errors.push(format!("delete {}/{}: {}", ent.name, id, e.message)),
            }
        }
    }

    // Invalidate every active session for the user. Even after the row is
    // gone, an in-flight session token would let a purged user keep acting.
    let revoked = ctx.session_store.revoke_all_for_user(user_id);

    let resp = serde_json::json!({
        "user_id": user_id,
        "rows_deleted": deleted,
        "sessions_revoked": revoked,
        "errors": errors,
        "purged_at": pylon_kernel::util::now_iso(),
    });
    (200, resp.to_string())
}

/// Gate a route behind admin auth. Returns `Some(error_response)` if the
/// caller is NOT admin, `None` if they are. Use at the top of any control-
/// plane handler (jobs, workflows, sync push, etc) that shouldn't be open
/// to arbitrary clients.
fn require_admin(ctx: &RouterContext) -> Option<(u16, String)> {
    if ctx.auth_ctx.is_admin {
        None
    } else {
        Some((
            403,
            json_error(
                "FORBIDDEN",
                "this endpoint requires admin auth (PYLON_ADMIN_TOKEN)",
            ),
        ))
    }
}

/// Gate a route behind "any authenticated identity". Returns `Some(err)` when
/// the caller has neither a user session nor an admin token. Used for the
/// rooms API, which previously let unauthenticated clients enumerate rooms
/// and read membership rosters — a silent presence-data leak.
fn require_auth(ctx: &RouterContext) -> Option<(u16, String)> {
    if ctx.auth_ctx.is_admin || ctx.auth_ctx.user_id.is_some() {
        None
    } else {
        Some((
            401,
            json_error("AUTH_REQUIRED", "authenticated session required"),
        ))
    }
}

pub fn json_error_with_hint(code: &str, message: &str, hint: &str) -> String {
    serde_json::json!({"error": {"code": code, "message": message, "hint": hint}}).to_string()
}

pub fn json_error_safe(code: &str, user_message: &str, internal: &str) -> String {
    tracing::warn!("[error] {code}: {internal}");
    json_error(code, user_message)
}

/// Parse a JSON request body, returning a 400 error tuple on failure.
fn parse_json(body: &str) -> Result<serde_json::Value, (u16, String)> {
    serde_json::from_str(body).map_err(|e| {
        (
            400,
            json_error_safe(
                "INVALID_JSON",
                "Invalid request body",
                &format!("Invalid JSON: {e}"),
            ),
        )
    })
}

/// Extract a query parameter value from a URL string.
fn query_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let search = format!("{key}=");
    url.split(&search).nth(1).and_then(|s| s.split('&').next())
}

fn chrono_now_iso() -> String {
    pylon_kernel::util::now_iso()
}

// ---------------------------------------------------------------------------
// Integration tests — auth-bypass regressions
// ---------------------------------------------------------------------------
//
// These tests lock in the auth-gate fixes from the security review so future
// changes can't silently re-introduce them. Each test builds a minimal
// RouterContext with stub implementations of the service traits and exercises
// a previously-vulnerable route.

#[cfg(test)]
mod auth_gate_tests {
    use super::*;
    use pylon_auth::{AuthContext, MagicCodeStore, OAuthStateStore, SessionStore};
    use pylon_kernel::{AppManifest, MANIFEST_VERSION};
    use pylon_policy::PolicyEngine;
    use pylon_sync::ChangeLog;

    // -----------------------------------------------------------------------
    // Minimal stubs for every service trait so we can build a RouterContext
    // without wiring up a real runtime.
    // -----------------------------------------------------------------------

    struct StubDataStore {
        manifest: AppManifest,
    }
    impl pylon_http::DataStore for StubDataStore {
        fn manifest(&self) -> &AppManifest {
            &self.manifest
        }
        fn insert(
            &self,
            _entity: &str,
            _data: &serde_json::Value,
        ) -> Result<String, pylon_http::DataError> {
            Ok("stub-id".to_string())
        }
        fn get_by_id(
            &self,
            _entity: &str,
            _id: &str,
        ) -> Result<Option<serde_json::Value>, pylon_http::DataError> {
            Ok(None)
        }
        fn list(&self, _entity: &str) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
            Ok(Vec::new())
        }
        fn list_after(
            &self,
            _entity: &str,
            _after: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
            Ok(Vec::new())
        }
        fn update(
            &self,
            _entity: &str,
            _id: &str,
            _data: &serde_json::Value,
        ) -> Result<bool, pylon_http::DataError> {
            Ok(true)
        }
        fn delete(&self, _entity: &str, _id: &str) -> Result<bool, pylon_http::DataError> {
            Ok(true)
        }
        fn lookup(
            &self,
            _entity: &str,
            _field: &str,
            _value: &str,
        ) -> Result<Option<serde_json::Value>, pylon_http::DataError> {
            Ok(None)
        }
        fn link(
            &self,
            _entity: &str,
            _id: &str,
            _relation: &str,
            _target_id: &str,
        ) -> Result<bool, pylon_http::DataError> {
            Ok(true)
        }
        fn unlink(
            &self,
            _entity: &str,
            _id: &str,
            _relation: &str,
        ) -> Result<bool, pylon_http::DataError> {
            Ok(true)
        }
        fn query_filtered(
            &self,
            _entity: &str,
            _filter: &serde_json::Value,
        ) -> Result<Vec<serde_json::Value>, pylon_http::DataError> {
            Ok(Vec::new())
        }
        fn query_graph(
            &self,
            _query: &serde_json::Value,
        ) -> Result<serde_json::Value, pylon_http::DataError> {
            Ok(serde_json::json!({}))
        }
        fn transact(
            &self,
            _ops: &[serde_json::Value],
        ) -> Result<(bool, Vec<serde_json::Value>), pylon_http::DataError> {
            Ok((true, Vec::new()))
        }
    }

    macro_rules! stub_ops {
        ($name:ident, $trait:path) => {
            struct $name;
        };
    }

    stub_ops!(StubRooms, RoomOps);
    stub_ops!(StubCache, CacheOps);
    stub_ops!(StubPubSub, PubSubOps);
    stub_ops!(StubJobs, JobOps);
    stub_ops!(StubScheduler, SchedulerOps);
    stub_ops!(StubWorkflows, WorkflowOps);
    stub_ops!(StubFiles, FileOps);
    stub_ops!(StubOpenApi, OpenApiGenerator);
    stub_ops!(StubEmail, EmailSender);

    impl RoomOps for StubRooms {
        fn join(
            &self,
            _room: &str,
            _user_id: &str,
            _data: Option<serde_json::Value>,
        ) -> Result<(serde_json::Value, serde_json::Value), pylon_http::DataError> {
            Ok((serde_json::json!({}), serde_json::json!({})))
        }
        fn leave(&self, _room: &str, _user_id: &str) -> Option<serde_json::Value> {
            None
        }
        fn set_presence(
            &self,
            _room: &str,
            _user_id: &str,
            _data: serde_json::Value,
        ) -> Option<serde_json::Value> {
            None
        }
        fn broadcast(
            &self,
            _room: &str,
            _sender: Option<&str>,
            _topic: &str,
            _data: serde_json::Value,
        ) -> Option<serde_json::Value> {
            None
        }
        fn list_rooms(&self) -> Vec<String> {
            vec![]
        }
        fn room_size(&self, _name: &str) -> usize {
            0
        }
        fn members(&self, _name: &str) -> Vec<serde_json::Value> {
            vec![]
        }
    }
    impl CacheOps for StubCache {
        fn handle_command(&self, _body: &str) -> (u16, String) {
            (200, "{}".into())
        }
        fn handle_get(&self, _key: &str) -> (u16, String) {
            (404, "{}".into())
        }
        fn handle_delete(&self, _key: &str) -> (u16, String) {
            (200, "{}".into())
        }
    }
    impl PubSubOps for StubPubSub {
        fn handle_publish(&self, _body: &str) -> (u16, String) {
            (200, "{}".into())
        }
        fn handle_channels(&self) -> (u16, String) {
            (200, "[]".into())
        }
        fn handle_history(&self, _channel: &str, _url: &str) -> (u16, String) {
            (200, "[]".into())
        }
    }
    impl JobOps for StubJobs {
        fn enqueue(
            &self,
            _name: &str,
            _payload: serde_json::Value,
            _priority: &str,
            _delay_secs: u64,
            _max_retries: u32,
            _queue: &str,
        ) -> String {
            "job-id".into()
        }
        fn stats(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn dead_letters(&self) -> serde_json::Value {
            serde_json::json!([])
        }
        fn retry_dead(&self, _id: &str) -> bool {
            false
        }
        fn list_jobs(
            &self,
            _status: Option<&str>,
            _queue: Option<&str>,
            _limit: usize,
        ) -> serde_json::Value {
            serde_json::json!([])
        }
        fn get_job(&self, _id: &str) -> Option<serde_json::Value> {
            None
        }
    }
    impl SchedulerOps for StubScheduler {
        fn list_tasks(&self) -> serde_json::Value {
            serde_json::json!([])
        }
        fn trigger(&self, _name: &str) -> bool {
            false
        }
    }
    impl WorkflowOps for StubWorkflows {
        fn definitions(&self) -> serde_json::Value {
            serde_json::json!([])
        }
        fn start(&self, _name: &str, _input: serde_json::Value) -> Result<String, String> {
            Ok("wf-id".into())
        }
        fn list(&self, _status: Option<&str>) -> serde_json::Value {
            serde_json::json!([])
        }
        fn get(&self, _id: &str) -> Option<serde_json::Value> {
            None
        }
        fn advance(&self, _id: &str) -> Result<String, String> {
            Ok("running".into())
        }
        fn send_event(
            &self,
            _id: &str,
            _event: &str,
            _data: serde_json::Value,
        ) -> Result<(), String> {
            Ok(())
        }
        fn cancel(&self, _id: &str) -> Result<(), String> {
            Ok(())
        }
    }
    impl FileOps for StubFiles {
        fn upload(&self, _body: &str) -> (u16, String) {
            (501, "{}".into())
        }
        fn get_file(&self, _id: &str) -> (u16, String) {
            (404, "{}".into())
        }
    }
    impl OpenApiGenerator for StubOpenApi {
        fn generate(&self, _base: &str) -> String {
            "{}".into()
        }
    }
    impl EmailSender for StubEmail {
        fn send(&self, _to: &str, _subject: &str, _body: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn empty_manifest() -> AppManifest {
        AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        }
    }

    /// Scaffold a RouterContext for tests. Caller chooses is_dev + auth.
    fn with_ctx<F>(is_dev: bool, auth: &AuthContext, f: F)
    where
        F: FnOnce(&RouterContext),
    {
        with_ctx_hooks(is_dev, auth, &NoopPluginHooks, f);
    }

    fn with_ctx_hooks<F>(is_dev: bool, auth: &AuthContext, hooks: &dyn PluginHookOps, f: F)
    where
        F: FnOnce(&RouterContext),
    {
        let manifest = empty_manifest();
        let store = StubDataStore {
            manifest: manifest.clone(),
        };
        let session_store = SessionStore::new();
        let magic_codes = MagicCodeStore::new();
        let oauth_state = OAuthStateStore::new();
        let policy_engine = PolicyEngine::from_manifest(&manifest);
        let change_log = ChangeLog::new();
        let notifier = NoopNotifier;
        let rooms = StubRooms;
        let cache = StubCache;
        let pubsub = StubPubSub;
        let jobs = StubJobs;
        let scheduler = StubScheduler;
        let workflows = StubWorkflows;
        let files = StubFiles;
        let openapi = StubOpenApi;
        let email = StubEmail;

        let ctx = RouterContext {
            store: &store,
            session_store: &session_store,
            magic_codes: &magic_codes,
            oauth_state: &oauth_state,
            policy_engine: &policy_engine,
            change_log: &change_log,
            notifier: &notifier,
            rooms: &rooms,
            cache: &cache,
            pubsub: &pubsub,
            jobs: &jobs,
            scheduler: &scheduler,
            workflows: &workflows,
            files: &files,
            openapi: &openapi,
            functions: None,
            email: &email,
            shards: None,
            plugin_hooks: hooks,
            auth_ctx: auth,
            is_dev,
            request_headers: &[],
        };
        f(&ctx);
    }

    // -----------------------------------------------------------------------
    // Regression tests for the previously-vulnerable routes
    // -----------------------------------------------------------------------

    /// Prior vuln: POST /api/auth/session accepted any user_id from anonymous
    /// callers and minted a valid session. Now: prod requires admin.
    #[test]
    fn auth_session_refuses_non_admin_in_prod() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/auth/session",
                r#"{"user_id":"victim"}"#,
                None,
            );
            assert_eq!(status, 403);
            assert!(body.contains("FORBIDDEN"));
        });
    }

    #[test]
    fn auth_session_allowed_for_admin_in_prod() {
        let admin = AuthContext::admin();
        with_ctx(false, &admin, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/auth/session",
                r#"{"user_id":"alice"}"#,
                None,
            );
            assert_eq!(status, 201);
        });
    }

    /// Prior vuln: OAuth callback accepted `{email, state}` without a real
    /// authorization code, letting anyone mint a session for any email.
    /// Now: prod requires an authorization code.
    #[test]
    fn oauth_callback_refuses_missing_code_in_prod() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            // Mint a state token so the state check passes — the `code`
            // requirement is what must still stop us.
            let state = ctx.oauth_state.create("google");
            let body = format!(r#"{{"state":"{state}","email":"victim@example.com"}}"#);
            let (status, resp, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/auth/callback/google",
                &body,
                None,
            );
            assert_eq!(status, 400);
            assert!(resp.contains("authorization") || resp.contains("code"));
        });
    }

    /// Prior vuln: /api/sync/push had no auth check.
    #[test]
    fn sync_push_requires_admin() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/sync/push",
                r#"{"changes":[]}"#,
                None,
            );
            assert_eq!(status, 403);
            assert!(body.contains("FORBIDDEN"));
        });
    }

    /// Prior vuln: /api/transact had no auth check.
    #[test]
    fn transact_requires_admin() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/transact",
                r#"{"ops":[]}"#,
                None,
            );
            assert_eq!(status, 403);
        });
    }

    /// Prior vuln: /api/workflows/start had no auth check.
    #[test]
    fn workflow_start_requires_admin() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/workflows/start",
                r#"{"name":"x"}"#,
                None,
            );
            assert_eq!(status, 403);
        });
    }

    /// Prior vuln: /api/jobs enqueue was open.
    #[test]
    fn jobs_enqueue_requires_admin() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/jobs",
                r#"{"name":"x","payload":{}}"#,
                None,
            );
            assert_eq!(status, 403);
        });
    }

    // -----------------------------------------------------------------------
    // Robustness / fuzz-style property tests
    //
    // The router is the one public entry point that takes arbitrary bytes
    // from the network. Any input must yield a response, not a panic.
    // These tests hammer the handlers with malformed bodies, weird paths,
    // and deeply-nested JSON. If any of them panic or loop, we catch it
    // here instead of in production.
    // -----------------------------------------------------------------------

    fn assert_route_doesnt_panic(ctx: &RouterContext, method: HttpMethod, url: &str, body: &str) {
        // `route` is synchronous. A panic would abort the test thread; the
        // test harness would fail the whole test. So just call it — success
        // means no panic.
        let (_status, _body, _ct) = route(ctx, method, url, body, None);
    }

    #[test]
    fn fuzz_malformed_json_bodies_never_panic() {
        let admin = AuthContext::admin();
        with_ctx(true, &admin, |ctx| {
            let samples = [
                "",
                "not json",
                "{",
                "}",
                "{\"",
                "{\"key\":",
                "[]",
                "null",
                "true",
                "\"string\"",
                "{\"changes\":\"not an array\"}",
                &format!("{{\"deeply\":{}}}", "{".repeat(1000)),
                "{\"unicode\":\"\\u0000\"}",
                "{\"numbers\":1e308}",
                "{\"negative\":-999999999999999}",
            ];
            for body in &samples {
                for url in &[
                    "/api/sync/push",
                    "/api/transact",
                    "/api/import",
                    "/api/batch",
                    "/api/jobs",
                    "/api/auth/session",
                    "/api/auth/magic/send",
                ] {
                    assert_route_doesnt_panic(ctx, HttpMethod::Post, url, body);
                }
            }
        });
    }

    #[test]
    fn fuzz_weird_urls_never_panic() {
        let admin = AuthContext::admin();
        with_ctx(true, &admin, |ctx| {
            let samples = [
                "/",
                "/api",
                "/api/",
                "/api/entities/",
                "/api/entities//",
                "/api/entities/%00",
                "/api/entities/../escape",
                "/api/entities/User?garbage=\x01",
                "/api/entities/User?$limit=abc&$order=garbage",
                &format!("/api/entities/{}", "a".repeat(10_000)),
                "/api/fn/",
                "/api/fn/traces",
                "/api/shards/id/connect",
                "/api/workflows/definitions",
                "/api/workflows/nonexistent/advance",
                "/api/rooms/",
                "/api/rooms/%20",
            ];
            for url in &samples {
                assert_route_doesnt_panic(ctx, HttpMethod::Get, url, "");
                assert_route_doesnt_panic(ctx, HttpMethod::Post, url, "{}");
                assert_route_doesnt_panic(ctx, HttpMethod::Delete, url, "");
            }
        });
    }

    #[test]
    fn fuzz_deeply_nested_json_dont_stack_overflow() {
        // serde_json has an internal recursion limit (default 128); confirm
        // depths beyond that return 400 rather than overflow the stack.
        let admin = AuthContext::admin();
        with_ctx(true, &admin, |ctx| {
            let depth = 300;
            let body = format!("{}{}", "[".repeat(depth), "]".repeat(depth),);
            let (status, _body, _ct) = route(ctx, HttpMethod::Post, "/api/sync/push", &body, None);
            // Serde may reject with 400, or the handler may accept and
            // treat as empty — either is fine. The key property: no panic.
            assert!(status >= 200 && status < 600);
        });
    }

    #[test]
    fn fuzz_unusual_http_methods_gracefully() {
        let admin = AuthContext::admin();
        with_ctx(true, &admin, |ctx| {
            for method in [
                HttpMethod::Get,
                HttpMethod::Post,
                HttpMethod::Put,
                HttpMethod::Patch,
                HttpMethod::Delete,
                HttpMethod::Options,
                HttpMethod::Head,
            ] {
                let (_status, _body, _ct) = route(ctx, method, "/api/entities/User", "{}", None);
            }
        });
    }

    /// Dev mode keeps the old permissive behaviour for local tooling.
    #[test]
    fn auth_session_allowed_in_dev_mode() {
        let anon = AuthContext::anonymous();
        with_ctx(true, &anon, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/auth/session",
                r#"{"user_id":"alice"}"#,
                None,
            );
            assert_eq!(status, 201);
        });
    }

    // -----------------------------------------------------------------------
    // Plugin CRUD hook wiring — prior vuln: POST/PATCH/DELETE on
    // /api/entities/* bypassed the registered plugin before_/after_ hooks,
    // so validation/audit_log/webhooks/slugify/timestamps never saw data-
    // plane writes. These tests pin that the router now runs them.
    // -----------------------------------------------------------------------

    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    struct CountingHooks {
        before_insert_calls: AtomicU32,
        after_insert_calls: AtomicU32,
        before_delete_calls: AtomicU32,
        reject_on_entity: Option<&'static str>,
    }

    impl CountingHooks {
        fn new() -> Self {
            Self {
                before_insert_calls: AtomicU32::new(0),
                after_insert_calls: AtomicU32::new(0),
                before_delete_calls: AtomicU32::new(0),
                reject_on_entity: None,
            }
        }
    }

    impl PluginHookOps for CountingHooks {
        fn before_insert(
            &self,
            entity: &str,
            _data: &mut serde_json::Value,
            _auth: &AuthContext,
        ) -> Result<(), (u16, String, String)> {
            self.before_insert_calls.fetch_add(1, Ordering::SeqCst);
            if self.reject_on_entity == Some(entity) {
                return Err((422, "VALIDATION".into(), "rejected by plugin".into()));
            }
            Ok(())
        }
        fn after_insert(
            &self,
            _entity: &str,
            _id: &str,
            _data: &serde_json::Value,
            _auth: &AuthContext,
        ) {
            self.after_insert_calls.fetch_add(1, Ordering::SeqCst);
        }
        fn before_update(
            &self,
            _entity: &str,
            _id: &str,
            _data: &mut serde_json::Value,
            _auth: &AuthContext,
        ) -> Result<(), (u16, String, String)> {
            Ok(())
        }
        fn after_update(
            &self,
            _entity: &str,
            _id: &str,
            _data: &serde_json::Value,
            _auth: &AuthContext,
        ) {
        }
        fn before_delete(
            &self,
            _entity: &str,
            _id: &str,
            _auth: &AuthContext,
        ) -> Result<(), (u16, String, String)> {
            self.before_delete_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn after_delete(&self, _entity: &str, _id: &str, _auth: &AuthContext) {}
    }

    #[test]
    fn plugin_hooks_fire_on_entity_post() {
        // StubDataStore::insert always succeeds with id="stub-id", so we
        // just assert the before_/after_ counters tick.
        let admin = AuthContext::admin();
        let hooks = CountingHooks::new();
        with_ctx_hooks(true, &admin, &hooks, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/entities/User",
                r#"{"email":"a@b"}"#,
                None,
            );
            assert_eq!(status, 201);
        });
        assert_eq!(hooks.before_insert_calls.load(Ordering::SeqCst), 1);
        assert_eq!(hooks.after_insert_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn plugin_before_insert_rejection_short_circuits_write() {
        // If the plugin rejects, the store's insert must NOT be called and
        // the status must propagate.
        let admin = AuthContext::admin();
        let rejector = CountingHooks {
            reject_on_entity: Some("User"),
            ..CountingHooks::new()
        };
        with_ctx_hooks(true, &admin, &rejector, |ctx| {
            let (status, body, _ct) =
                route(ctx, HttpMethod::Post, "/api/entities/User", r#"{}"#, None);
            assert_eq!(status, 422);
            assert!(body.contains("VALIDATION"));
        });
        assert_eq!(rejector.before_insert_calls.load(Ordering::SeqCst), 1);
        // after_insert must NOT have been called when before_insert rejected.
        assert_eq!(rejector.after_insert_calls.load(Ordering::SeqCst), 0);
    }

    // -----------------------------------------------------------------------
    // GDPR export + purge — pinned behavior so accidental breakage shows up
    // as a test failure instead of a compliance surprise.
    // -----------------------------------------------------------------------

    #[test]
    fn gdpr_export_requires_admin() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/admin/users/alice/export",
                "",
                None,
            );
            assert_eq!(status, 403);
            assert!(body.contains("FORBIDDEN"));
        });
    }

    #[test]
    fn gdpr_purge_requires_admin() {
        let anon = AuthContext::anonymous();
        with_ctx(false, &anon, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Delete,
                "/api/admin/users/alice/purge",
                "",
                None,
            );
            assert_eq!(status, 403);
        });
    }

    #[test]
    fn gdpr_export_returns_envelope_for_admin() {
        let admin = AuthContext::admin();
        with_ctx(true, &admin, |ctx| {
            let (status, body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/admin/users/alice/export",
                "",
                None,
            );
            assert_eq!(status, 200);
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(v["user_id"], "alice");
            assert!(v["entities"].is_object());
            assert!(v["exported_at"].is_string());
        });
    }

    #[test]
    fn plugin_hooks_fire_on_entity_delete() {
        let admin = AuthContext::admin();
        let hooks = CountingHooks::new();
        with_ctx_hooks(true, &admin, &hooks, |ctx| {
            let (status, _body, _ct) = route(
                ctx,
                HttpMethod::Delete,
                "/api/entities/User/stub-id",
                "",
                None,
            );
            assert_eq!(status, 200);
        });
        assert_eq!(hooks.before_delete_calls.load(Ordering::SeqCst), 1);
    }

    // -----------------------------------------------------------------------
    // Webhook endpoint — /api/webhooks/:action
    //
    // No functions are registered in the stub RouterContext, so we can only
    // assert the error paths (FUNCTIONS_NOT_AVAILABLE, FN_NOT_FOUND). The
    // happy-path with request-context propagation is covered by the TS
    // runtime tests + a manual integration check against a live server.
    // -----------------------------------------------------------------------

    #[test]
    fn webhook_returns_503_when_functions_unavailable() {
        let anon = AuthContext::anonymous();
        with_ctx(true, &anon, |ctx| {
            let (status, body, _ct) = route(
                ctx,
                HttpMethod::Post,
                "/api/webhooks/stripe_handler",
                "{}",
                None,
            );
            assert_eq!(status, 503);
            assert!(body.contains("FUNCTIONS_NOT_AVAILABLE"));
        });
    }

    #[test]
    fn webhook_accepts_any_http_method() {
        // Regression: providers send GET challenge requests (e.g. Slack URL
        // verification). The endpoint must route regardless of method.
        let anon = AuthContext::anonymous();
        with_ctx(true, &anon, |ctx| {
            for method in [
                HttpMethod::Get,
                HttpMethod::Post,
                HttpMethod::Put,
                HttpMethod::Patch,
                HttpMethod::Delete,
            ] {
                let (status, _body, _ct) = route(ctx, method, "/api/webhooks/any_name", "", None);
                // Without function runtime it's 503 regardless; the point
                // is that we didn't 405 Method Not Allowed.
                assert_ne!(status, 405);
            }
        });
    }

    // Keep the warning silencer until this is used.
    #[allow(dead_code)]
    const _TOUCH_ATOMIC_BOOL: AtomicBool = AtomicBool::new(false);
}
