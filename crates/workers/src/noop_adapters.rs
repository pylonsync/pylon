//! Adapters for router traits on Cloudflare Workers.
//!
//! Real implementations land here as Workers bindings get wired:
//!
//! - **OpenApiGenerator**: real manifest-derived spec (this file).
//!   Same shape the runtime emits; entities → CRUD paths, actions →
//!   POST /api/actions/X, queries → POST /api/query/X.
//! - **SchedulerOps**: lists cron tasks declared in the manifest's
//!   `routes` (cron-triggered routes). Workers cron handlers fire
//!   these via the `scheduled` event — see handler.rs.
//! - **CacheOps**: real KV-backed impl available as `KvCache` (see
//!   `crates/workers/src/kv_cache.rs`) — handler.rs picks it up
//!   from `env.kv("PYLON_CACHE")` when the binding exists. The
//!   noop in this file is the fallback when no binding's wired,
//!   returning typed `KV_BINDING_REQUIRED` 503s so operators can
//!   wrangler.toml-debug from the error.
//! - **FileOps**: real R2-backed impl available as `R2Files` (see
//!   `crates/workers/src/r2_files.rs`) — handler.rs picks it up
//!   from `env.bucket("PYLON_FILES")` when the binding exists. The
//!   noop is the fallback (typed `R2_BINDING_REQUIRED` 503s).
//! - **RoomOps / PubSubOps / JobOps / WorkflowOps**: still 503
//!   stubs. These want Durable Objects / Queues / Workflows
//!   bindings — heavier integration than KV / R2 because the
//!   stateful adapters need a DO class to host the state.
//!
//! The error codes are typed (KV_BINDING_REQUIRED, R2_BINDING_REQUIRED,
//! DO_BINDING_REQUIRED, WORKFLOWS_BINDING_REQUIRED) so operators
//! can wrangler.toml-debug from the error alone instead of guessing
//! what's missing.

use pylon_http::DataError;
use pylon_router::{
    CacheOps, FileOps, JobOps, OpenApiGenerator, PubSubOps, RoomOps, SchedulerOps, WorkflowOps,
};

/// Implements all router service traits with stub responses where
/// platform-specific bindings haven't been wired yet, and REAL
/// responses for everything derivable purely from the manifest.
pub struct NoopAll {
    manifest: pylon_kernel::AppManifest,
}

impl NoopAll {
    pub fn new(manifest: &pylon_kernel::AppManifest) -> Self {
        Self {
            manifest: manifest.clone(),
        }
    }
}

impl RoomOps for NoopAll {
    fn join(
        &self,
        _room: &str,
        _user_id: &str,
        _data: Option<serde_json::Value>,
    ) -> Result<(serde_json::Value, serde_json::Value), DataError> {
        // Pylon's rooms map to one Durable Object per room (sticky
        // routing + per-DO WebSocket fan-out). The DO class +
        // wrangler.toml binding hasn't been wired into the Workers
        // target yet — add `[[durable_objects.bindings]]` entries
        // pointing at a `PylonRoom` DO once that class lands.
        Err(DataError {
            code: "DO_BINDING_REQUIRED".into(),
            message: "Rooms on Workers need a Durable Object binding (PylonRoom) — \
                      not yet implemented in this target. Use the Fly / self-host \
                      runtime for rooms today."
                .into(),
        })
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

impl CacheOps for NoopAll {
    fn handle_command(&self, _body: &str) -> (u16, String) {
        (
            503,
            pylon_router::json_error(
                "KV_BINDING_REQUIRED",
                "Cache on Workers needs a KV binding (PYLON_CACHE) — not yet wired \
                 in this target; add `[[kv_namespaces]]` in wrangler.toml and the \
                 worker handler will populate this adapter from env.kv(\"PYLON_CACHE\").",
            ),
        )
    }

    fn handle_get(&self, _key: &str) -> (u16, String) {
        (
            503,
            pylon_router::json_error(
                "KV_BINDING_REQUIRED",
                "Cache on Workers needs a KV binding (PYLON_CACHE) — not yet wired \
                 in this target; add `[[kv_namespaces]]` in wrangler.toml and the \
                 worker handler will populate this adapter from env.kv(\"PYLON_CACHE\").",
            ),
        )
    }

    fn handle_delete(&self, _key: &str) -> (u16, String) {
        (
            503,
            pylon_router::json_error(
                "KV_BINDING_REQUIRED",
                "Cache on Workers needs a KV binding (PYLON_CACHE) — not yet wired \
                 in this target; add `[[kv_namespaces]]` in wrangler.toml and the \
                 worker handler will populate this adapter from env.kv(\"PYLON_CACHE\").",
            ),
        )
    }
}

impl PubSubOps for NoopAll {
    fn handle_publish(&self, _body: &str) -> (u16, String) {
        (
            503,
            pylon_router::json_error(
                "DO_BINDING_REQUIRED",
                "PubSub on Workers needs a Durable Object binding (PylonPubSub) — \
                 not yet wired in this target. The DO holds connected subscribers \
                 and fans publish events out via WebSocket.",
            ),
        )
    }

    fn handle_channels(&self) -> (u16, String) {
        (200, "[]".into())
    }

    fn handle_history(&self, _channel: &str, _url: &str) -> (u16, String) {
        (200, "[]".into())
    }
}

impl JobOps for NoopAll {
    /// Workers job execution wants a Cloudflare Queues binding
    /// (`env.queue("PYLON_JOBS")`). Until that lands, enqueue
    /// silently no-ops — return an empty id so the caller knows
    /// the enqueue didn't take. (Real impl will return the
    /// queue-assigned message id.)
    fn enqueue(
        &self,
        _name: &str,
        _payload: serde_json::Value,
        _priority: &str,
        _delay_secs: u64,
        _max_retries: u32,
        _queue: &str,
    ) -> String {
        String::new()
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

impl SchedulerOps for NoopAll {
    /// Workers cron triggers are declared in `wrangler.toml`
    /// `[triggers] crons = [...]` and fire via the `scheduled` event
    /// — the schedule isn't in the Pylon manifest, so we can't list
    /// concrete tasks here. Return an empty list (dashboard tools
    /// degrade to "no scheduled tasks"); a future revision could
    /// surface the cron expressions via an env var.
    fn list_tasks(&self) -> serde_json::Value {
        serde_json::json!([])
    }

    /// `trigger` would dispatch a cron task on demand, but Workers
    /// crons are unilaterally scheduled by Cloudflare's edge —
    /// there's no in-process way to fire one. Return false so
    /// operators don't think a manual trigger landed.
    fn trigger(&self, _name: &str) -> bool {
        false
    }
}

// Workflows on Workers stays on the noop path because the
// `worker` crate (0.5 at time of writing) doesn't surface a
// Workflows binding API. Cloudflare Workflows is post-crate-
// version; once `worker::Workflow` lands, swap this impl for a
// real WorkersWorkflows adapter that wires
// env.workflow("PYLON_WORKFLOWS") through. Until then, every
// WorkflowOps call returns the typed WORKFLOWS_BINDING_REQUIRED
// error so customers see a clear "not supported on this target"
// signal instead of a silent hang.
impl WorkflowOps for NoopAll {
    fn definitions(&self) -> serde_json::Value {
        // List the workflow names declared in the manifest's actions
        // so dashboard tools can at least discover the surface, even
        // when the Workers target can't execute them yet. Real
        // execution needs the Cloudflare Workflows binding (separate
        // product from the function runner; would land here as
        // env.workflow("PYLON_WORKFLOWS")).
        let names: Vec<serde_json::Value> = self
            .manifest
            .actions
            .iter()
            .map(|a| serde_json::json!({"name": a.name}))
            .collect();
        serde_json::Value::Array(names)
    }

    fn start(&self, _name: &str, _input: serde_json::Value) -> Result<String, String> {
        Err(
            "WORKFLOWS_BINDING_REQUIRED: Cloudflare Workflows binding not wired in this target"
                .into(),
        )
    }

    fn list(&self, _status_filter: Option<&str>) -> serde_json::Value {
        serde_json::json!([])
    }

    fn get(&self, _id: &str) -> Option<serde_json::Value> {
        None
    }

    fn advance(&self, _id: &str) -> Result<String, String> {
        Err(
            "WORKFLOWS_BINDING_REQUIRED: Cloudflare Workflows binding not wired in this target"
                .into(),
        )
    }

    fn send_event(&self, _id: &str, _event: &str, _data: serde_json::Value) -> Result<(), String> {
        Err(
            "WORKFLOWS_BINDING_REQUIRED: Cloudflare Workflows binding not wired in this target"
                .into(),
        )
    }

    fn cancel(&self, _id: &str) -> Result<(), String> {
        Err(
            "WORKFLOWS_BINDING_REQUIRED: Cloudflare Workflows binding not wired in this target"
                .into(),
        )
    }
}

impl FileOps for NoopAll {
    fn upload(&self, _body: &str) -> (u16, String) {
        (
            503,
            pylon_router::json_error(
                "R2_BINDING_REQUIRED",
                "File uploads on Workers need an R2 bucket binding (PYLON_FILES) — \
                 not yet wired in this target; add `[[r2_buckets]]` in wrangler.toml \
                 and the worker handler will populate this adapter from \
                 env.bucket(\"PYLON_FILES\").",
            ),
        )
    }

    fn get_file(
        &self,
        _id: &str,
        _requester_user_id: Option<&str>,
        _is_admin: bool,
    ) -> (u16, String) {
        (
            503,
            pylon_router::json_error(
                "R2_BINDING_REQUIRED",
                "File downloads on Workers need an R2 bucket binding (PYLON_FILES) — \
                 not yet wired in this target.",
            ),
        )
    }
}

impl OpenApiGenerator for NoopAll {
    /// Build a real OpenAPI 3.0.3 spec from the manifest. Mirrors the
    /// runtime crate's generator at the path level: one CRUD bundle
    /// per entity, one POST per action, one POST per query (the
    /// filtered-query route shape). Pre-fix this returned an empty
    /// `paths` object — clients couldn't discover anything via the
    /// /openapi.json endpoint when pylon was deployed on Workers.
    ///
    /// Schema-level details (request/response models per entity)
    /// are deferred; the path list is the load-bearing piece for
    /// dashboard tools that probe the surface, and the runtime
    /// emits a richer spec for that target. Workers can catch up
    /// once a shared pylon-openapi crate lands.
    fn generate(&self, base_url: &str) -> String {
        let mut paths = serde_json::Map::new();

        for entity in &self.manifest.entities {
            // /api/entities/<name> — list + create
            let list_create = serde_json::json!({
                "get": {
                    "summary": format!("List {} rows", entity.name),
                    "responses": {"200": {"description": "OK"}},
                },
                "post": {
                    "summary": format!("Create a {} row", entity.name),
                    "responses": {"201": {"description": "Created"}},
                },
            });
            paths.insert(format!("/api/entities/{}", entity.name), list_create);
            // /api/entities/<name>/<id> — get + update + delete
            let item = serde_json::json!({
                "get": {
                    "summary": format!("Get one {} row", entity.name),
                    "parameters": [
                        {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}},
                    ],
                    "responses": {"200": {"description": "OK"}, "404": {"description": "Not found"}},
                },
                "patch": {
                    "summary": format!("Update a {} row", entity.name),
                    "parameters": [
                        {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}},
                    ],
                    "responses": {"200": {"description": "OK"}, "404": {"description": "Not found"}},
                },
                "delete": {
                    "summary": format!("Delete a {} row", entity.name),
                    "parameters": [
                        {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}},
                    ],
                    "responses": {"204": {"description": "Deleted"}, "404": {"description": "Not found"}},
                },
            });
            paths.insert(format!("/api/entities/{}/{{id}}", entity.name), item);
            // /api/query/<name> — filtered query
            paths.insert(
                format!("/api/query/{}", entity.name),
                serde_json::json!({
                    "post": {
                        "summary": format!("Filtered query against {}", entity.name),
                        "responses": {"200": {"description": "OK"}},
                    },
                }),
            );
        }

        for action in &self.manifest.actions {
            paths.insert(
                format!("/api/actions/{}", action.name),
                serde_json::json!({
                    "post": {
                        "summary": format!("Invoke action {}", action.name),
                        "responses": {"200": {"description": "OK"}},
                    },
                }),
            );
        }

        for query in &self.manifest.queries {
            paths.insert(
                format!("/api/fn/{}", query.name),
                serde_json::json!({
                    "post": {
                        "summary": format!("Run query function {}", query.name),
                        "responses": {"200": {"description": "OK"}},
                    },
                }),
            );
        }

        serde_json::json!({
            "openapi": "3.0.3",
            "info": {
                "title": self.manifest.name,
                "version": self.manifest.version,
            },
            "servers": [{"url": base_url}],
            "paths": paths,
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::{AppManifest, ManifestAction, ManifestEntity, ManifestField, ManifestQuery};

    fn manifest_with_one_of_each() -> AppManifest {
        AppManifest {
            manifest_version: pylon_kernel::MANIFEST_VERSION,
            name: "test-app".into(),
            version: "1.2.3".into(),
            entities: vec![ManifestEntity {
                name: "Note".into(),
                fields: vec![ManifestField {
                    name: "id".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                }],
                ..Default::default()
            }],
            actions: vec![ManifestAction {
                name: "publishNote".into(),
                input: vec![],
            }],
            queries: vec![ManifestQuery {
                name: "listNotes".into(),
                input: vec![],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn openapi_derives_crud_routes_per_entity() {
        let manifest = manifest_with_one_of_each();
        let adapter = NoopAll::new(&manifest);
        let spec_str = adapter.generate("https://app.example.com");
        let spec: serde_json::Value = serde_json::from_str(&spec_str).unwrap();
        // Entity CRUD bundle.
        assert!(spec["paths"]["/api/entities/Note"]["get"].is_object());
        assert!(spec["paths"]["/api/entities/Note"]["post"].is_object());
        assert!(spec["paths"]["/api/entities/Note/{id}"]["get"].is_object());
        assert!(spec["paths"]["/api/entities/Note/{id}"]["patch"].is_object());
        assert!(spec["paths"]["/api/entities/Note/{id}"]["delete"].is_object());
        // Filtered query route.
        assert!(spec["paths"]["/api/query/Note"]["post"].is_object());
        // Action surfaces.
        assert!(spec["paths"]["/api/actions/publishNote"]["post"].is_object());
        // Query-function surfaces.
        assert!(spec["paths"]["/api/fn/listNotes"]["post"].is_object());
        // info echoes the manifest.
        assert_eq!(spec["info"]["title"], "test-app");
        assert_eq!(spec["info"]["version"], "1.2.3");
        // servers carries the caller-supplied base URL.
        assert_eq!(spec["servers"][0]["url"], "https://app.example.com");
    }

    #[test]
    fn openapi_empty_manifest_produces_empty_paths() {
        let manifest = AppManifest {
            manifest_version: pylon_kernel::MANIFEST_VERSION,
            name: "empty".into(),
            version: "0".into(),
            ..Default::default()
        };
        let adapter = NoopAll::new(&manifest);
        let spec: serde_json::Value = serde_json::from_str(&adapter.generate("https://x")).unwrap();
        assert_eq!(spec["paths"].as_object().unwrap().len(), 0);
        // Spec still well-formed (openapi version + info present).
        assert_eq!(spec["openapi"], "3.0.3");
        assert_eq!(spec["info"]["title"], "empty");
    }

    #[test]
    fn workflow_definitions_list_manifest_actions() {
        // Workers can't EXECUTE workflows yet but the discovery
        // surface should at least list what's declared.
        let manifest = manifest_with_one_of_each();
        let adapter = NoopAll::new(&manifest);
        let defs = adapter.definitions();
        let arr = defs.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "publishNote");
    }

    #[test]
    fn typed_error_codes_point_at_the_missing_binding() {
        // Operators reading the response body should be able to
        // wrangler.toml-debug from the error code alone.
        let manifest = AppManifest {
            manifest_version: pylon_kernel::MANIFEST_VERSION,
            name: "x".into(),
            version: "0".into(),
            ..Default::default()
        };
        let adapter = NoopAll::new(&manifest);
        let (status, body) = adapter.handle_get("k");
        assert_eq!(status, 503);
        assert!(body.contains("KV_BINDING_REQUIRED"));
        let (status, body) = adapter.handle_publish("{}");
        assert_eq!(status, 503);
        assert!(body.contains("DO_BINDING_REQUIRED"));
        let (status, body) = adapter.upload("");
        assert_eq!(status, 503);
        assert!(body.contains("R2_BINDING_REQUIRED"));
        let err = adapter.start("any", serde_json::json!({})).unwrap_err();
        assert!(err.contains("WORKFLOWS_BINDING_REQUIRED"));
        let join_err = adapter.join("r1", "u1", None).unwrap_err();
        assert_eq!(join_err.code, "DO_BINDING_REQUIRED");
    }
}
