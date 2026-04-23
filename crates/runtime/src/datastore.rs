//! Implements the platform-agnostic [`DataStore`] trait for [`Runtime`].
//!
//! This bridges the concrete SQLite-backed Runtime to the abstract trait
//! used by the router crate, enabling the same routing logic to run on
//! self-hosted servers and Cloudflare Workers alike.

use pylon_http::{DataError, DataStore};

use crate::Runtime;

// ---------------------------------------------------------------------------
// DataStore → Runtime bridge
// ---------------------------------------------------------------------------

impl DataStore for Runtime {
    fn manifest(&self) -> &pylon_kernel::AppManifest {
        Runtime::manifest(self)
    }

    fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, DataError> {
        Runtime::insert(self, entity, data).map_err(into_data_error)
    }

    fn get_by_id(
        &self,
        entity: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, DataError> {
        Runtime::get_by_id(self, entity, id).map_err(into_data_error)
    }

    fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, DataError> {
        Runtime::list(self, entity).map_err(into_data_error)
    }

    fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        Runtime::list_after(self, entity, after, limit).map_err(into_data_error)
    }

    fn update(
        &self,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
    ) -> Result<bool, DataError> {
        Runtime::update(self, entity, id, data).map_err(into_data_error)
    }

    fn delete(&self, entity: &str, id: &str) -> Result<bool, DataError> {
        Runtime::delete(self, entity, id).map_err(into_data_error)
    }

    fn lookup(
        &self,
        entity: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<serde_json::Value>, DataError> {
        Runtime::lookup(self, entity, field, value).map_err(into_data_error)
    }

    fn link(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, DataError> {
        Runtime::link(self, entity, id, relation, target_id).map_err(into_data_error)
    }

    fn unlink(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
    ) -> Result<bool, DataError> {
        Runtime::unlink(self, entity, id, relation).map_err(into_data_error)
    }

    fn query_filtered(
        &self,
        entity: &str,
        filter: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        Runtime::query_filtered(self, entity, filter).map_err(into_data_error)
    }

    fn query_graph(
        &self,
        query: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        Runtime::query_graph(self, query).map_err(into_data_error)
    }

    fn aggregate(
        &self,
        entity: &str,
        spec: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        Runtime::aggregate(self, entity, spec).map_err(into_data_error)
    }

    fn transact(
        &self,
        ops: &[serde_json::Value],
    ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
        let conn = self.lock_conn_pub().map_err(into_data_error)?;
        let _ = conn.execute("BEGIN", []);
        let mut results: Vec<serde_json::Value> = Vec::new();
        let mut rollback = false;

        for op in ops {
            let op_type = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let entity = op.get("entity").and_then(|v| v.as_str()).unwrap_or("");

            match op_type {
                "insert" => {
                    let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                    match self.insert_with_conn(&conn, entity, &data) {
                        Ok(id) => {
                            results.push(serde_json::json!({"op": "insert", "id": id}));
                        }
                        Err(e) => {
                            results.push(serde_json::json!({"op": "insert", "error": e.message}));
                            rollback = true;
                            break;
                        }
                    }
                }
                "update" => {
                    let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                    match self.update_with_conn(&conn, entity, id, &data) {
                        Ok(_) => {
                            results.push(serde_json::json!({"op": "update", "id": id}));
                        }
                        Err(e) => {
                            results.push(serde_json::json!({"op": "update", "error": e.message}));
                            rollback = true;
                            break;
                        }
                    }
                }
                "delete" => {
                    let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    match self.delete_with_conn(&conn, entity, id) {
                        Ok(_) => {
                            results.push(serde_json::json!({"op": "delete", "id": id}));
                        }
                        Err(e) => {
                            results.push(serde_json::json!({"op": "delete", "error": e.message}));
                            rollback = true;
                            break;
                        }
                    }
                }
                _ => {
                    results
                        .push(serde_json::json!({"op": op_type, "error": "unknown operation"}));
                }
            }
        }

        if rollback {
            let _ = conn.execute("ROLLBACK", []);
        } else {
            let _ = conn.execute("COMMIT", []);
        }

        Ok((!rollback, results))
    }
}

fn into_data_error(e: crate::RuntimeError) -> DataError {
    DataError {
        code: e.code,
        message: e.message,
    }
}

// ---------------------------------------------------------------------------
// ChangeNotifier for WsHub + SseHub
// ---------------------------------------------------------------------------

use crate::sse::SseHub;
use crate::ws::WsHub;
use std::sync::Arc;

/// Bridges WebSocket + SSE hubs to the router's [`ChangeNotifier`] trait.
pub struct WsSseNotifier {
    pub ws: Arc<WsHub>,
    pub sse: Arc<SseHub>,
}

impl pylon_router::ChangeNotifier for WsSseNotifier {
    fn notify(&self, event: &pylon_sync::ChangeEvent) {
        self.ws.broadcast(event);
        self.sse.broadcast(event);
    }

    fn notify_presence(&self, json: &str) {
        self.ws.broadcast_presence(json);
        self.sse.broadcast_message(json);
    }
}

/// Serialize a value to JSON, falling back to `{}` on failure.
fn to_json<T: serde::Serialize>(val: T) -> serde_json::Value {
    serde_json::to_value(val).unwrap_or(serde_json::json!({}))
}

/// Serialize a value to JSON, falling back to `[]` on failure.
fn to_json_array<T: serde::Serialize>(val: T) -> serde_json::Value {
    serde_json::to_value(val).unwrap_or(serde_json::json!([]))
}

// ---------------------------------------------------------------------------
// Adapter: RoomManager → RoomOps
// ---------------------------------------------------------------------------

use crate::rooms::RoomManager;

impl pylon_router::RoomOps for RoomManager {
    fn join(
        &self,
        room: &str,
        user_id: &str,
        data: Option<serde_json::Value>,
    ) -> Result<(serde_json::Value, serde_json::Value), DataError> {
        RoomManager::join(self, room, user_id, data)
            .map(|(snapshot, join_event)| {
                (
                    to_json(&snapshot),
                    to_json(&join_event),
                )
            })
            .map_err(|e| DataError {
                code: e.code,
                message: e.message,
            })
    }

    fn leave(&self, room: &str, user_id: &str) -> Option<serde_json::Value> {
        RoomManager::leave(self, room, user_id)
            .map(|event| to_json(&event))
    }

    fn set_presence(
        &self,
        room: &str,
        user_id: &str,
        data: serde_json::Value,
    ) -> Option<serde_json::Value> {
        RoomManager::set_presence(self, room, user_id, data)
            .map(|event| to_json(&event))
    }

    fn broadcast(
        &self,
        room: &str,
        sender: Option<&str>,
        topic: &str,
        data: serde_json::Value,
    ) -> Option<serde_json::Value> {
        RoomManager::broadcast(self, room, sender, topic, data)
            .map(|event| to_json(&event))
    }

    fn list_rooms(&self) -> Vec<String> {
        RoomManager::list_rooms(self)
    }

    fn room_size(&self, name: &str) -> usize {
        RoomManager::room_size(self, name)
    }

    fn members(&self, name: &str) -> Vec<serde_json::Value> {
        RoomManager::members(self, name)
            .into_iter()
            .map(|p| to_json(p))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Adapter: CachePlugin → CacheOps (newtype wrapper for orphan rule)
// ---------------------------------------------------------------------------

use pylon_plugin::builtin::cache::CachePlugin;

/// Adapter that routes router-level CRUD hook calls into the PluginRegistry.
///
/// The router holds a `&dyn PluginHookOps`; this adapter wraps the runtime's
/// `Arc<PluginRegistry>` so registered plugins (audit_log, validation,
/// webhooks, timestamps, slugify, versioning, search) run on every
/// POST/PATCH/DELETE under `/api/entities/*`. Without this wiring, plugins
/// only saw the `on_request` hook and never got a chance to observe or
/// reject data-plane writes — a quiet correctness hole noted in the
/// pentest review.
pub struct PluginHooksAdapter(pub Arc<pylon_plugin::PluginRegistry>);

impl pylon_router::PluginHookOps for PluginHooksAdapter {
    fn before_insert(
        &self,
        entity: &str,
        data: &mut serde_json::Value,
        auth: &pylon_auth::AuthContext,
    ) -> Result<(), (u16, String, String)> {
        self.0
            .run_before_insert(entity, data, auth)
            .map_err(|e| (e.status, e.code, e.message))
    }
    fn after_insert(
        &self,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
        auth: &pylon_auth::AuthContext,
    ) {
        self.0.run_after_insert(entity, id, data, auth);
    }
    fn before_update(
        &self,
        entity: &str,
        id: &str,
        data: &mut serde_json::Value,
        auth: &pylon_auth::AuthContext,
    ) -> Result<(), (u16, String, String)> {
        self.0
            .run_before_update(entity, id, data, auth)
            .map_err(|e| (e.status, e.code, e.message))
    }
    fn after_update(
        &self,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
        auth: &pylon_auth::AuthContext,
    ) {
        self.0.run_after_update(entity, id, data, auth);
    }
    fn before_delete(
        &self,
        entity: &str,
        id: &str,
        auth: &pylon_auth::AuthContext,
    ) -> Result<(), (u16, String, String)> {
        self.0
            .run_before_delete(entity, id, auth)
            .map_err(|e| (e.status, e.code, e.message))
    }
    fn after_delete(&self, entity: &str, id: &str, auth: &pylon_auth::AuthContext) {
        self.0.run_after_delete(entity, id, auth);
    }
}

pub struct CacheAdapter(pub Arc<CachePlugin>);

impl pylon_router::CacheOps for CacheAdapter {
    fn handle_command(&self, body: &str) -> (u16, String) {
        crate::cache_handlers::handle_cache_command(&self.0, body)
    }

    fn handle_get(&self, key: &str) -> (u16, String) {
        crate::cache_handlers::handle_cache_get(&self.0, key)
    }

    fn handle_delete(&self, key: &str) -> (u16, String) {
        crate::cache_handlers::handle_cache_delete(&self.0, key)
    }
}

// ---------------------------------------------------------------------------
// Adapter: PubSubBroker → PubSubOps (newtype wrapper for orphan rule)
// ---------------------------------------------------------------------------

use crate::pubsub::PubSubBroker;

pub struct PubSubAdapter(pub Arc<PubSubBroker>);

impl pylon_router::PubSubOps for PubSubAdapter {
    fn handle_publish(&self, body: &str) -> (u16, String) {
        crate::cache_handlers::handle_pubsub_publish(&self.0, body)
    }

    fn handle_channels(&self) -> (u16, String) {
        crate::cache_handlers::handle_pubsub_channels(&self.0)
    }

    fn handle_history(&self, channel: &str, url: &str) -> (u16, String) {
        crate::cache_handlers::handle_pubsub_history(&self.0, channel, url)
    }
}

// ---------------------------------------------------------------------------
// Adapter: JobQueue → JobOps
// ---------------------------------------------------------------------------

use crate::jobs::{JobQueue, Priority};

impl pylon_router::JobOps for JobQueue {
    fn enqueue(
        &self,
        name: &str,
        payload: serde_json::Value,
        priority: &str,
        delay_secs: u64,
        max_retries: u32,
        queue: &str,
    ) -> String {
        let pri = Priority::from_str_loose(priority);
        JobQueue::enqueue_with_options(self, name, payload, pri, delay_secs, max_retries, queue)
    }

    fn stats(&self) -> serde_json::Value {
        to_json(JobQueue::stats(self))
    }

    fn dead_letters(&self) -> serde_json::Value {
        to_json_array(JobQueue::dead_letters(self))
    }

    fn retry_dead(&self, id: &str) -> bool {
        JobQueue::retry_dead(self, id)
    }

    fn list_jobs(
        &self,
        status: Option<&str>,
        queue: Option<&str>,
        limit: usize,
    ) -> serde_json::Value {
        to_json_array(JobQueue::list_jobs(self, status, queue, limit))
    }

    fn get_job(&self, id: &str) -> Option<serde_json::Value> {
        JobQueue::get_job(self, id)
            .map(|j| to_json(j))
    }
}

// ---------------------------------------------------------------------------
// Adapter: Scheduler → SchedulerOps
// ---------------------------------------------------------------------------

use crate::scheduler::Scheduler;

impl pylon_router::SchedulerOps for Scheduler {
    fn list_tasks(&self) -> serde_json::Value {
        to_json_array(Scheduler::list_tasks(self))
    }

    fn trigger(&self, name: &str) -> bool {
        Scheduler::trigger(self, name)
    }
}

// ---------------------------------------------------------------------------
// Adapter: WorkflowEngine → WorkflowOps
// ---------------------------------------------------------------------------

use crate::workflows::WorkflowEngine;

impl pylon_router::WorkflowOps for WorkflowEngine {
    fn definitions(&self) -> serde_json::Value {
        to_json_array(WorkflowEngine::definitions(self))
    }

    fn start(&self, name: &str, input: serde_json::Value) -> Result<String, String> {
        WorkflowEngine::start(self, name, input)
    }

    fn list(&self, status_filter: Option<&str>) -> serde_json::Value {
        // Convert string filter to WorkflowStatus for the engine.
        let filter = status_filter.and_then(|s| match s {
            "pending" => Some(crate::workflows::WorkflowStatus::Pending),
            "running" => Some(crate::workflows::WorkflowStatus::Running),
            "sleeping" => Some(crate::workflows::WorkflowStatus::Sleeping),
            "waiting" => Some(crate::workflows::WorkflowStatus::WaitingForEvent),
            "completed" => Some(crate::workflows::WorkflowStatus::Completed),
            "failed" => Some(crate::workflows::WorkflowStatus::Failed),
            "cancelled" => Some(crate::workflows::WorkflowStatus::Cancelled),
            _ => None,
        });
        to_json_array(WorkflowEngine::list(self, filter.as_ref()))
    }

    fn get(&self, id: &str) -> Option<serde_json::Value> {
        WorkflowEngine::get(self, id)
            .map(|inst| to_json(inst))
    }

    fn advance(&self, id: &str) -> Result<String, String> {
        WorkflowEngine::advance(self, id).map(|status| format!("{:?}", status))
    }

    fn send_event(
        &self,
        id: &str,
        event: &str,
        data: serde_json::Value,
    ) -> Result<(), String> {
        WorkflowEngine::send_event(self, id, event, data)
    }

    fn cancel(&self, id: &str) -> Result<(), String> {
        WorkflowEngine::cancel(self, id)
    }
}

// ---------------------------------------------------------------------------
// Adapter: FileStorage trait → FileOps
// ---------------------------------------------------------------------------

use pylon_storage::files::{FileStorage, LocalFileStorage};

/// Adapter that exposes a [`FileStorage`] backend through the router's [`FileOps`].
pub struct FileOpsAdapter {
    pub storage: Arc<dyn FileStorage>,
}

impl FileOpsAdapter {
    /// Create from environment variables.
    /// Defaults to local filesystem storage at `./uploads`.
    pub fn from_env() -> Self {
        let dir = std::env::var("PYLON_FILES_DIR").unwrap_or_else(|_| "uploads".into());
        let url_prefix =
            std::env::var("PYLON_FILES_URL_PREFIX").unwrap_or_else(|_| "/api/files".into());
        Self {
            storage: Arc::new(LocalFileStorage::new(&dir, &url_prefix)),
        }
    }
}

impl pylon_router::FileOps for FileOpsAdapter {
    fn upload(&self, _body: &str) -> (u16, String) {
        // The self-hosted server short-circuits /api/files/upload BEFORE the
        // request body is lossily coerced to a String, so binary uploads are
        // handled there. This fallback exists for non-self-hosted adapters
        // (e.g., Workers) and for defense in depth; it rejects string bodies
        // that wouldn't carry binary data correctly.
        (
            400,
            pylon_router::json_error(
                "UPLOAD_NEEDS_BINARY",
                "File uploads must use multipart/form-data or raw binary with X-Filename; this platform does not support string-body uploads",
            ),
        )
    }

    fn get_file(&self, id: &str) -> (u16, String) {
        match self.storage.get(id) {
            Ok(content) => (200, String::from_utf8_lossy(&content).into_owned()),
            Err(e) if e.code == "NOT_FOUND" => {
                (404, pylon_router::json_error("FILE_NOT_FOUND", &e.message))
            }
            Err(e) => (400, pylon_router::json_error(&e.code, &e.message)),
        }
    }
}

/// Backwards-compatible alias; old code refers to this name.
pub type LocalFileOps = FileOpsAdapter;

impl LocalFileOps {
    /// Default instance backed by the local `uploads/` directory.
    pub fn new_default() -> Self {
        Self::from_env()
    }
}

// ---------------------------------------------------------------------------
// Adapter: EmailTransport → EmailSender
// ---------------------------------------------------------------------------

use pylon_auth::email::{ConsoleTransport, EmailTransport, HttpEmailTransport};

/// Picks an email backend based on environment variables.
/// Falls back to `ConsoleTransport` (prints to stderr) when no provider is configured.
pub struct EmailAdapter {
    transport: Box<dyn EmailTransport>,
}

impl EmailAdapter {
    pub fn from_env() -> Self {
        if let Some(http) = HttpEmailTransport::from_env() {
            Self {
                transport: Box::new(http),
            }
        } else {
            Self {
                transport: Box::new(ConsoleTransport),
            }
        }
    }
}

impl pylon_router::EmailSender for EmailAdapter {
    fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String> {
        self.transport
            .send(to, subject, body)
            .map_err(|e| e.message)
    }
}

// ---------------------------------------------------------------------------
// Adapter: OpenAPI generator
// ---------------------------------------------------------------------------

pub struct RuntimeOpenApiGenerator<'a> {
    pub manifest: &'a pylon_kernel::AppManifest,
}

impl<'a> pylon_router::OpenApiGenerator for RuntimeOpenApiGenerator<'a> {
    fn generate(&self, base_url: &str) -> String {
        let spec = crate::openapi::generate_openapi(self.manifest, base_url);
        serde_json::to_string(&spec).unwrap_or_else(|_| "{}".into())
    }
}

// ---------------------------------------------------------------------------
// Adapter: DynShardRegistry → ShardOps
// ---------------------------------------------------------------------------

/// Wraps any `Arc<dyn DynShardRegistry>` so the router can dispatch shard
/// routes without knowing the concrete SimState type.
pub struct ShardOpsAdapter {
    pub registry: Arc<dyn pylon_realtime::DynShardRegistry>,
}

impl pylon_router::ShardOps for ShardOpsAdapter {
    fn get_shard(&self, id: &str) -> Option<Arc<dyn pylon_realtime::DynShard>> {
        self.registry.get(id)
    }

    fn list_shards(&self) -> Vec<String> {
        self.registry.ids()
    }

    fn shard_count(&self) -> usize {
        self.registry.len()
    }
}

#[cfg(test)]
mod find_runtime_tests {
    use super::*;

    #[test]
    fn env_override_takes_precedence() {
        let dir = std::env::temp_dir().join(format!("pylon_rt_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("custom_runtime.ts");
        std::fs::write(&path, "// test").unwrap();

        std::env::set_var("PYLON_FUNCTIONS_RUNTIME", path.to_str().unwrap());
        let found = find_functions_runtime();
        std::env::remove_var("PYLON_FUNCTIONS_RUNTIME");

        assert_eq!(found.as_deref(), path.to_str());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn returns_none_when_env_path_missing() {
        std::env::set_var(
            "PYLON_FUNCTIONS_RUNTIME",
            "/tmp/definitely-does-not-exist-42.ts",
        );
        // May still find something in CWD (dev path), so we only assert the env
        // path isn't what gets returned.
        let found = find_functions_runtime();
        std::env::remove_var("PYLON_FUNCTIONS_RUNTIME");
        assert_ne!(
            found.as_deref(),
            Some("/tmp/definitely-does-not-exist-42.ts")
        );
    }
}

// ---------------------------------------------------------------------------
// TxStore — DataStore backed by a held transaction connection
// ---------------------------------------------------------------------------

/// A `DataStore` that executes against a pre-held SQLite connection
/// for the duration of a single mutation handler.
///
/// # Safety contract
///
/// `rusqlite::Connection` is `Send` but not `Sync` (it uses `RefCell`s
/// internally for statement caching). The `DataStore` trait requires
/// `Send + Sync`, but `&'a Connection` is neither.
///
/// We hand-implement both via `unsafe impl` because:
///
/// 1. **Construction.** `TxStore::new` is only ever called by
///    `FnOpsImpl::call` for mutations, after acquiring the runtime's
///    write lock. The `&Connection` originates from a `MutexGuard`
///    that the constructing thread holds.
///
/// 2. **Lifetime.** The `'a` lifetime ties the `TxStore` to that guard.
///    The compiler enforces that the `TxStore` cannot outlive the held
///    lock; it must be dropped before the guard is.
///
/// 3. **Single-threaded use.** `FnRunner::call()` runs the handler
///    synchronously on the calling thread and never spawns threads
///    holding a reference to the `TxStore`. The `Send + Sync` bounds
///    on the `DataStore` trait are satisfied vacuously — no thread
///    other than the caller ever sees this `TxStore`.
///
/// 4. **No interior aliasing.** All `&Connection` calls go through
///    `Runtime::*_with_conn` methods which take `&Connection`, never
///    keeping the reference alive across an `await` point (this is
///    sync code, no awaits).
///
/// Future work: refactor `Runtime`'s `write_conn` to be
/// `Arc<Mutex<Connection>>` so TxStore can hold an `Arc<Mutex<...>>`,
/// eliminating the unsafe impl entirely.
pub struct TxStore<'a> {
    runtime: &'a Runtime,
    conn: &'a rusqlite::Connection,
    /// Pending change events to broadcast after the outer transaction
    /// commits. Buffered here rather than pushed to ChangeLog + notifier
    /// immediately so a rollback doesn't emit events for writes that
    /// didn't actually land.
    pending: std::cell::RefCell<Vec<pylon_sync::ChangeEvent>>,
}

impl<'a> TxStore<'a> {
    pub fn new(runtime: &'a Runtime, conn: &'a rusqlite::Connection) -> Self {
        Self {
            runtime,
            conn,
            pending: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Drain the pending-events buffer. Called after COMMIT succeeds;
    /// the caller is responsible for appending each event to the
    /// ChangeLog and broadcasting via the notifier. On rollback the
    /// caller just drops the buffer without calling this.
    pub fn take_pending(&self) -> Vec<pylon_sync::ChangeEvent> {
        std::mem::take(&mut *self.pending.borrow_mut())
    }

    fn record(
        &self,
        entity: &str,
        row_id: &str,
        kind: pylon_sync::ChangeKind,
        data: Option<&serde_json::Value>,
    ) {
        self.pending
            .borrow_mut()
            .push(pylon_sync::ChangeEvent {
                seq: 0, // assigned by ChangeLog::append after commit
                entity: entity.to_string(),
                row_id: row_id.to_string(),
                kind,
                data: data.cloned(),
                timestamp: String::new(),
            });
    }
}

// SAFETY: see the contract on TxStore above.
unsafe impl<'a> Sync for TxStore<'a> {}
unsafe impl<'a> Send for TxStore<'a> {}

impl<'a> DataStore for TxStore<'a> {
    fn manifest(&self) -> &pylon_kernel::AppManifest {
        self.runtime.manifest()
    }

    fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, DataError> {
        let id = self
            .runtime
            .insert_with_conn(self.conn, entity, data)
            .map_err(into_data_error)?;
        // Buffer the event. If the outer mutation rolls back, the buffer
        // is dropped instead of flushed, so sync subscribers never see a
        // row that doesn't exist.
        self.record(entity, &id, pylon_sync::ChangeKind::Insert, Some(data));
        Ok(id)
    }

    fn get_by_id(
        &self,
        entity: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, DataError> {
        self.runtime
            .get_by_id_with_conn(self.conn, entity, id)
            .map_err(into_data_error)
    }

    fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, DataError> {
        self.runtime
            .list_with_conn(self.conn, entity)
            .map_err(into_data_error)
    }

    fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        self.runtime
            .list_after_with_conn(self.conn, entity, after, limit)
            .map_err(into_data_error)
    }

    fn update(
        &self,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
    ) -> Result<bool, DataError> {
        let updated = self
            .runtime
            .update_with_conn(self.conn, entity, id, data)
            .map_err(into_data_error)?;
        if updated {
            self.record(entity, id, pylon_sync::ChangeKind::Update, Some(data));
        }
        Ok(updated)
    }

    fn delete(&self, entity: &str, id: &str) -> Result<bool, DataError> {
        let deleted = self
            .runtime
            .delete_with_conn(self.conn, entity, id)
            .map_err(into_data_error)?;
        if deleted {
            self.record(entity, id, pylon_sync::ChangeKind::Delete, None);
        }
        Ok(deleted)
    }

    fn lookup(
        &self,
        entity: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<serde_json::Value>, DataError> {
        self.runtime
            .lookup_with_conn(self.conn, entity, field, value)
            .map_err(into_data_error)
    }

    fn link(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, DataError> {
        self.runtime
            .link_with_conn(self.conn, entity, id, relation, target_id)
            .map_err(into_data_error)
    }

    fn unlink(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
    ) -> Result<bool, DataError> {
        self.runtime
            .unlink_with_conn(self.conn, entity, id, relation)
            .map_err(into_data_error)
    }

    fn query_filtered(
        &self,
        entity: &str,
        filter: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, DataError> {
        self.runtime
            .query_filtered_with_conn(self.conn, entity, filter)
            .map_err(into_data_error)
    }

    fn query_graph(
        &self,
        query: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        self.runtime
            .query_graph_with_conn(self.conn, query)
            .map_err(into_data_error)
    }

    fn aggregate(
        &self,
        entity: &str,
        spec: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        // Aggregation inside a transaction uses the same runtime method.
        // The lookups do their own read-lock, which is fine since aggregate
        // is read-only.
        Runtime::aggregate(self.runtime, entity, spec).map_err(into_data_error)
    }

    fn transact(
        &self,
        _ops: &[serde_json::Value],
    ) -> Result<(bool, Vec<serde_json::Value>), DataError> {
        // Nested transactions aren't supported from within a mutation handler.
        // The mutation handler IS the transaction.
        Err(DataError {
            code: "NESTED_TRANSACTION".into(),
            message: "ctx.db.transact() is not allowed inside a mutation handler (the handler itself is transactional)".into(),
        })
    }
}

// ---------------------------------------------------------------------------
// Adapter: FnRunner → FnOps
// ---------------------------------------------------------------------------

use pylon_functions::protocol::{AuthInfo as FnAuth, FnType};
use pylon_functions::registry::{FnDef, FnRegistry};
use pylon_functions::runner::{FnCallError, FnRunner};
use pylon_functions::trace::FnTrace;

/// Adapter that implements [`FnOps`] by delegating to a [`FnRunner`].
///
/// Holds an `Arc<Runtime>` so function handlers get a [`DataStore`] to
/// operate against.
pub struct FnOpsImpl {
    pub runner: Arc<FnRunner>,
    pub registry: Arc<FnRegistry>,
    pub runtime: Arc<Runtime>,
    /// Per-function rate limiter, keyed on `"<fn_name>::<identity>"`.
    /// Limits are uniform; per-fn overrides can be added later via FnDef
    /// metadata once the TS define API surfaces them.
    pub fn_rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    /// Sync change log for broadcasting `ctx.db.insert/update/delete` ops
    /// that happen inside a function handler. Without this, mutations via
    /// functions silently bypass sync — WS subscribers see nothing until
    /// they manually refetch. Flushed post-COMMIT so rollbacks don't emit
    /// phantom events.
    pub change_log: Arc<pylon_sync::ChangeLog>,
    /// Where to broadcast change events after a function mutation commits.
    pub notifier: Arc<dyn pylon_router::ChangeNotifier>,
}

impl pylon_router::FnOps for FnOpsImpl {
    fn get_fn(&self, name: &str) -> Option<FnDef> {
        self.registry.get(name)
    }

    fn list_fns(&self) -> Vec<FnDef> {
        self.registry.list()
    }

    fn call(
        &self,
        fn_name: &str,
        args: serde_json::Value,
        auth: FnAuth,
        on_stream: Option<Box<dyn FnMut(&str) + Send>>,
        request: Option<pylon_functions::protocol::RequestInfo>,
    ) -> Result<(serde_json::Value, FnTrace), FnCallError> {
        let def = self.registry.get(fn_name).ok_or_else(|| FnCallError {
            code: "FN_NOT_FOUND".into(),
            message: format!("Function \"{fn_name}\" is not registered"),
        })?;

        match def.fn_type {
            FnType::Mutation => {
                // Hold the write connection for the entire handler duration.
                // This keeps the BEGIN/COMMIT span free of interleaving from
                // other writers (who would otherwise become part of the
                // transaction because SQLite tracks it on the connection).
                //
                // Inside the handler, every `ctx.db` call routes through
                // TxStore, which uses this same held connection — so no
                // re-locking, no deadlock, no interleaving.
                let conn_guard = self.runtime.lock_conn_pub().map_err(|e| FnCallError {
                    code: e.code,
                    message: e.message,
                })?;

                if let Err(e) = conn_guard.execute("BEGIN", []) {
                    return Err(FnCallError {
                        code: "BEGIN_FAILED".into(),
                        message: format!("Failed to start transaction: {e}"),
                    });
                }

                let tx_store = TxStore::new(&self.runtime, &conn_guard);
                let result = self.runner.call(
                    &tx_store,
                    fn_name,
                    def.fn_type,
                    args,
                    auth,
                    on_stream,
                    request,
                );

                // Surface commit/rollback errors. A swallowed COMMIT failure
                // is the worst possible outcome: the caller sees success but
                // the data isn't durable. A swallowed ROLLBACK failure leaves
                // the connection in an unknown txn state for the next caller.
                let result = match result {
                    Ok(value) => match conn_guard.execute("COMMIT", []) {
                        Ok(_) => {
                            // Flush buffered change events NOW — after the
                            // commit durably lands but before we return
                            // success. Ordering matters: append to the log
                            // first (so /api/sync/pull callers that race
                            // with this broadcast see the row in the tail),
                            // then notify WS/SSE subscribers. `seq` on each
                            // pending event starts at 0; append assigns
                            // the real seq.
                            for ev in tx_store.take_pending() {
                                let seq = self.change_log.append(
                                    &ev.entity,
                                    &ev.row_id,
                                    ev.kind.clone(),
                                    ev.data.clone(),
                                );
                                let event = pylon_sync::ChangeEvent { seq, ..ev };
                                self.notifier.notify(&event);
                            }
                            Ok(value)
                        }
                        Err(commit_err) => {
                            // Best-effort cleanup. If ROLLBACK also fails the
                            // connection is in a bad state — at minimum the
                            // operator sees both failures in the log.
                            if let Err(rollback_err) = conn_guard.execute("ROLLBACK", []) {
                                tracing::warn!(
                                    "[functions] ROLLBACK after COMMIT failure also failed: {rollback_err}"
                                );
                            }
                            Err(FnCallError {
                                code: "COMMIT_FAILED".into(),
                                message: format!(
                                    "Function \"{fn_name}\" succeeded but COMMIT failed: {commit_err}"
                                ),
                            })
                        }
                    },
                    Err(handler_err) => {
                        if let Err(rollback_err) = conn_guard.execute("ROLLBACK", []) {
                            // Don't shadow the handler error — log the
                            // rollback failure separately.
                            tracing::warn!(
                                "[functions] ROLLBACK after handler error failed: {rollback_err}"
                            );
                        }
                        Err(handler_err)
                    }
                };
                // conn_guard drops here, releasing the lock.
                result
            }
            _ => self.runner.call(
                &*self.runtime,
                fn_name,
                def.fn_type,
                args,
                auth,
                on_stream,
                request,
            ),
        }
    }

    fn recent_traces(&self, limit: usize) -> Vec<FnTrace> {
        self.runner.trace_log.recent(limit)
    }

    fn check_rate_limit(&self, fn_name: &str, identity: &str) -> Result<(), u64> {
        let key = format!("{fn_name}::{identity}");
        self.fn_rate_limiter.check(&key)
    }
}

/// Spawn the Bun function runtime if a `functions/` directory exists.
///
/// Returns `Some(FnOpsImpl)` if successful, `None` if no functions directory
/// or if Bun is not installed. Errors during startup print to stderr and
/// return `None` to keep the server running.
/// Resolve the path to the TypeScript function runtime script.
///
/// Searches in order:
/// 1. `$PYLON_FUNCTIONS_RUNTIME` environment variable (if set and file exists)
/// 2. `./node_modules/@pylon/functions/src/runtime.ts` (npm-installed)
/// 3. `./node_modules/@pylon/functions/dist/runtime.js` (built)
/// 4. `~/.pylon/runtime.ts` (user install)
/// 5. `packages/functions/src/runtime.ts` (dev monorepo)
///
/// Returns `None` if none exist.
pub fn find_functions_runtime() -> Option<String> {
    if let Ok(env_path) = std::env::var("PYLON_FUNCTIONS_RUNTIME") {
        if std::path::Path::new(&env_path).exists() {
            return Some(env_path);
        }
    }

    // Walk parent directories like Node.js resolution does, so running
    // `pylon dev` from an example sub-directory still finds the
    // hoisted workspace package at the repo root. Without this, bun/npm
    // workspace users see "TypeScript function runtime is not configured"
    // and think the server is broken when it's just a CWD issue.
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let relative_candidates = [
        "node_modules/@pylon/functions/src/runtime.ts",
        "node_modules/@pylon/functions/dist/runtime.js",
        // Monorepo dev: source tree at the workspace root.
        "packages/functions/src/runtime.ts",
    ];

    let mut dir: Option<&std::path::Path> = Some(cwd.as_path());
    while let Some(current) = dir {
        for rel in &relative_candidates {
            let candidate = current.join(rel);
            if candidate.exists() {
                return candidate.to_str().map(|s| s.to_string());
            }
        }
        dir = current.parent();
    }

    // Final fallback: user-wide install under ~/.pylon.
    let user_path = format!("{home}/.pylon/runtime.ts");
    if std::path::Path::new(&user_path).exists() {
        return Some(user_path);
    }
    None
}

pub fn try_spawn_functions(
    runtime: Arc<Runtime>,
    job_queue: Arc<crate::jobs::JobQueue>,
    fn_rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    change_log: Arc<pylon_sync::ChangeLog>,
    notifier: Arc<dyn pylon_router::ChangeNotifier>,
) -> Option<Arc<FnOpsImpl>> {
    let fn_dir = std::env::var("PYLON_FUNCTIONS_DIR").unwrap_or_else(|_| "functions".into());
    if !std::path::Path::new(&fn_dir).exists() {
        return None;
    }

    let runtime_script = match find_functions_runtime() {
        Some(p) => p,
        None => {
            tracing::warn!(
                "[functions] No TypeScript runtime script found. TypeScript functions will be unavailable."
            );
            tracing::warn!(
                "[functions] Tried: $PYLON_FUNCTIONS_RUNTIME, node_modules/@pylon/functions/src/runtime.ts, ~/.pylon/runtime.ts, packages/functions/src/runtime.ts"
            );
            return None;
        }
    };

    let runner = Arc::new(FnRunner::new(1000));

    // start() now performs the handshake itself and returns the function
    // definitions, so there's no separate handshake step. On any failure the
    // child has already been killed.
    let defs = match runner.start("bun", &["run", &runtime_script, &fn_dir]) {
        Ok(defs) => defs,
        Err(e) => {
            tracing::warn!("[functions] Failed to start Bun runtime: {e}");
            tracing::warn!(
                "[functions] Install Bun from https://bun.sh — TypeScript functions will be unavailable."
            );
            return None;
        }
    };

    // Wire scheduler requests from functions into the job queue. Use the
    // Result-returning variant so a persist failure surfaces as a TS-side
    // SCHEDULE_FAILED error instead of `{scheduled:true, id:""}`.
    runner.set_schedule_hook(Box::new(move |fn_name, args, delay_ms, run_at| {
        let delay_secs = match (delay_ms, run_at) {
            (Some(ms), _) => ms / 1000,
            (None, Some(ts)) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                if ts > now {
                    (ts - now) / 1000
                } else {
                    0
                }
            }
            _ => 0,
        };
        job_queue.try_enqueue_with_options(
            fn_name,
            args,
            crate::jobs::Priority::Normal,
            delay_secs,
            3,
            "functions",
        )
    }));

    let registry = Arc::new(FnRegistry::new());
    let count = defs.len();
    registry.replace_all(defs);
    tracing::warn!("[functions] Loaded {count} function(s) from {fn_dir}");

    let ops = Arc::new(FnOpsImpl {
        runner,
        registry,
        runtime,
        fn_rate_limiter,
        change_log,
        notifier,
    });

    install_nested_call_hook(&ops);
    spawn_runtime_supervisor(Arc::clone(&ops));
    Some(ops)
}

/// Route nested `RunFn` calls (action → query/mutation) through a
/// transactional wrapper so nested mutations get their own BEGIN/COMMIT.
///
/// Uses a `Weak<FnOpsImpl>` to avoid keeping the ops struct alive forever
/// through a cycle (hook stored on FnRunner ← held by FnOpsImpl). When the
/// ops struct is dropped the hook becomes a no-op error.
fn install_nested_call_hook(ops: &Arc<FnOpsImpl>) {
    use pylon_functions::protocol::{AuthInfo, FnType};

    let weak = Arc::downgrade(ops);
    ops.runner.set_nested_call_hook(Box::new(
        move |fn_name: &str,
              fn_type: FnType,
              args: serde_json::Value,
              auth: AuthInfo|
              -> Result<serde_json::Value, (String, String)> {
            let ops = match weak.upgrade() {
                Some(o) => o,
                None => {
                    return Err((
                        "RUNTIME_GONE".into(),
                        "pylon runtime is shutting down".into(),
                    ))
                }
            };

            match fn_type {
                FnType::Mutation => {
                    // Wrap the nested mutation in its own write-conn + BEGIN
                    // + COMMIT, matching the top-level mutation contract.
                    let conn_guard =
                        ops.runtime.lock_conn_pub().map_err(|e| (e.code, e.message))?;
                    if let Err(e) = conn_guard.execute("BEGIN", []) {
                        return Err(("BEGIN_FAILED".into(), e.to_string()));
                    }
                    let tx_store = TxStore::new(&ops.runtime, &conn_guard);
                    // Re-enter protocol without acquiring io_lock — we're
                    // already inside the outer call_inner which holds it.
                    // Nested calls never get HTTP request metadata — that's
                    // only meaningful for the top-level webhook invocation.
                    let result = ops.runner.call_inner(
                        &tx_store,
                        fn_name,
                        fn_type,
                        args,
                        auth,
                        None,
                        None,
                    );
                    match result {
                        Ok((value, _trace)) => {
                            if let Err(e) = conn_guard.execute("COMMIT", []) {
                                let _ = conn_guard.execute("ROLLBACK", []);
                                return Err(("COMMIT_FAILED".into(), e.to_string()));
                            }
                            // Flush change events after COMMIT so nested
                            // mutations (action → runMutation(...)) broadcast
                            // the same way top-level mutations do. Without
                            // this, every write an action emits is invisible
                            // to sync subscribers until the NEXT top-level
                            // mutation lands — streaming UIs stay empty.
                            for ev in tx_store.take_pending() {
                                let seq = ops.change_log.append(
                                    &ev.entity,
                                    &ev.row_id,
                                    ev.kind.clone(),
                                    ev.data.clone(),
                                );
                                let event =
                                    pylon_sync::ChangeEvent { seq, ..ev };
                                ops.notifier.notify(&event);
                            }
                            Ok(value)
                        }
                        Err(e) => {
                            let _ = conn_guard.execute("ROLLBACK", []);
                            Err((e.code, e.message))
                        }
                    }
                }
                _ => {
                    // Queries + actions: no transaction wrap needed. Just
                    // re-enter protocol via the same store (runtime).
                    // Nested: no HTTP request propagated (see above).
                    let result = ops.runner.call_inner(
                        &*ops.runtime,
                        fn_name,
                        fn_type,
                        args,
                        auth,
                        None,
                        None,
                    );
                    result.map(|(v, _)| v).map_err(|e| (e.code, e.message))
                }
            }
        },
    ));
}

/// Background watchdog that restarts the Bun runtime if it dies (crashed,
/// killed by the call timeout path, OOM, etc.). Exponential backoff: 1s, 2s,
/// 4s, ... capped at 30s. Resets to 1s after a successful respawn.
///
/// We don't try to "give up" — if Bun keeps crashing the supervisor keeps
/// trying with the capped delay. The operator sees repeated WARN logs and
/// can investigate. Better than silently leaving functions disabled forever.
fn spawn_runtime_supervisor(ops: Arc<FnOpsImpl>) {
    use std::time::Duration;

    std::thread::Builder::new()
        .name("pylon-fn-supervisor".into())
        .spawn(move || {
            let mut backoff = Duration::from_secs(1);
            let max_backoff = Duration::from_secs(30);
            loop {
                std::thread::sleep(Duration::from_secs(2));
                if ops.runner.is_alive() {
                    backoff = Duration::from_secs(1);
                    continue;
                }
                tracing::warn!(
                    "[functions] Bun runtime is not alive — respawning after {:?}",
                    backoff
                );
                std::thread::sleep(backoff);
                match ops.runner.respawn() {
                    Ok(defs) => {
                        let count = defs.len();
                        // Replace, not merge — deleted functions must stop
                        // being callable. register_all() alone leaves stale
                        // entries from the previous process generation.
                        ops.registry.replace_all(defs);
                        tracing::warn!("[functions] Respawned Bun runtime ({count} fn(s))");
                        backoff = Duration::from_secs(1);
                    }
                    Err(e) => {
                        tracing::warn!("[functions] Respawn failed: {e}");
                        // Persistent Bun-runtime failures are the kind of
                        // operator signal that belongs in error telemetry
                        // too. Include enough context to triage repeated
                        // events: current backoff (so operators can see
                        // how long failures have been compounding) and the
                        // component name.
                        let backoff_str = format!("{}", backoff.as_secs());
                        pylon_observability::report_error(
                            &pylon_observability::ErrorEvent {
                                level: pylon_observability::ErrorLevel::Error,
                                code: "FN_RESPAWN_FAILED",
                                message: &e,
                                context: &[
                                    ("component", "bun-runtime-supervisor"),
                                    ("backoff_secs", &backoff_str),
                                ],
                            },
                        );
                        backoff = (backoff * 2).min(max_backoff);
                    }
                }
            }
        })
        .expect("failed to spawn function runtime supervisor");
}
