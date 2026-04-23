//! No-op adapters for router traits not yet supported on Workers.
//!
//! These implement the router's trait interfaces with stub responses.
//! As Workers features are built out (Durable Object rooms, Queues jobs,
//! etc.), these will be replaced with real implementations.

use pylon_http::DataError;
use pylon_router::{
    CacheOps, FileOps, JobOps, OpenApiGenerator, PubSubOps, RoomOps, SchedulerOps, WorkflowOps,
};

/// Implements all router service traits with no-op stubs.
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
        Err(DataError {
            code: "NOT_AVAILABLE".into(),
            message: "Rooms are not available on this platform".into(),
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
            pylon_router::json_error("NOT_AVAILABLE", "Cache not available on this platform"),
        )
    }

    fn handle_get(&self, _key: &str) -> (u16, String) {
        (
            503,
            pylon_router::json_error("NOT_AVAILABLE", "Cache not available on this platform"),
        )
    }

    fn handle_delete(&self, _key: &str) -> (u16, String) {
        (
            503,
            pylon_router::json_error("NOT_AVAILABLE", "Cache not available on this platform"),
        )
    }
}

impl PubSubOps for NoopAll {
    fn handle_publish(&self, _body: &str) -> (u16, String) {
        (
            503,
            pylon_router::json_error("NOT_AVAILABLE", "PubSub not available on this platform"),
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
    fn list_tasks(&self) -> serde_json::Value {
        serde_json::json!([])
    }

    fn trigger(&self, _name: &str) -> bool {
        false
    }
}

impl WorkflowOps for NoopAll {
    fn definitions(&self) -> serde_json::Value {
        serde_json::json!([])
    }

    fn start(&self, _name: &str, _input: serde_json::Value) -> Result<String, String> {
        Err("Workflows not available on this platform".into())
    }

    fn list(&self, _status_filter: Option<&str>) -> serde_json::Value {
        serde_json::json!([])
    }

    fn get(&self, _id: &str) -> Option<serde_json::Value> {
        None
    }

    fn advance(&self, _id: &str) -> Result<String, String> {
        Err("Workflows not available on this platform".into())
    }

    fn send_event(&self, _id: &str, _event: &str, _data: serde_json::Value) -> Result<(), String> {
        Err("Workflows not available on this platform".into())
    }

    fn cancel(&self, _id: &str) -> Result<(), String> {
        Err("Workflows not available on this platform".into())
    }
}

impl FileOps for NoopAll {
    fn upload(&self, _body: &str) -> (u16, String) {
        (
            503,
            pylon_router::json_error(
                "NOT_AVAILABLE",
                "File uploads not available on this platform",
            ),
        )
    }

    fn get_file(&self, _id: &str) -> (u16, String) {
        (
            503,
            pylon_router::json_error(
                "NOT_AVAILABLE",
                "File storage not available on this platform",
            ),
        )
    }
}

impl OpenApiGenerator for NoopAll {
    fn generate(&self, _base_url: &str) -> String {
        serde_json::json!({
            "openapi": "3.0.3",
            "info": {
                "title": self.manifest.name,
                "version": self.manifest.version,
            },
            "paths": {}
        })
        .to_string()
    }
}
