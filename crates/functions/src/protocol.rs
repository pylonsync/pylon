//! Bidirectional NDJSON protocol between Rust runtime and TypeScript process.
//!
//! Messages are newline-delimited JSON objects. Each function invocation gets
//! a unique `call_id` for multiplexing concurrent calls over a single connection.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Rust → TypeScript messages
// ---------------------------------------------------------------------------

/// Invoke a function on the TypeScript side.
#[derive(Debug, Clone, Serialize)]
pub struct CallMessage {
    #[serde(rename = "type")]
    pub msg_type: &'static str, // always "call"
    pub call_id: String,
    pub fn_name: String,
    pub fn_type: FnType,
    pub args: serde_json::Value,
    pub auth: AuthInfo,
    /// HTTP request context — present only when the action is invoked via
    /// a custom HTTP route (`defineRoute` binding). Actions called from
    /// other actions via `ctx.runAction` or from jobs don't get this.
    /// Enables Stripe-webhook-style signature verification + access to
    /// raw headers/body the router would otherwise discard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestInfo>,
}

/// HTTP request metadata forwarded to TypeScript actions invoked via
/// `defineRoute` bindings. All fields are strings so the TS side can use
/// them directly without re-parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestInfo {
    /// Uppercased method — `POST`, `GET`, etc.
    pub method: String,
    /// Full request path (with query string if any).
    pub path: String,
    /// Lowercased header names → values. Multi-value headers are joined
    /// with `, ` per RFC 7230. This trades some fidelity for a map shape
    /// that's ergonomic to consume from TS.
    pub headers: std::collections::HashMap<String, String>,
    /// The exact bytes of the request body, UTF-8-decoded. Webhook
    /// signature verification (Stripe, GitHub) needs the bytes that were
    /// signed, so this is NOT the parsed JSON.
    pub raw_body: String,
}

impl CallMessage {
    pub fn new(
        call_id: String,
        fn_name: String,
        fn_type: FnType,
        args: serde_json::Value,
        auth: AuthInfo,
    ) -> Self {
        Self {
            msg_type: "call",
            call_id,
            fn_name,
            fn_type,
            args,
            auth,
            request: None,
        }
    }

    /// Attach HTTP request metadata (used when the call originated from a
    /// `defineRoute` HTTP binding rather than a programmatic invocation).
    pub fn with_request(mut self, request: RequestInfo) -> Self {
        self.request = Some(request);
        self
    }
}

/// Result of a DB operation, sent back to TypeScript.
///
/// `op_id` is echoed from the incoming `DbOpMessage.op_id` when present.
/// The TS runtime uses it to demux concurrent DB ops inside a single
/// function call (e.g. `Promise.all([ctx.db.get(a), ctx.db.get(b)])`).
/// Absent `op_id` keeps legacy TS runtimes compatible.
#[derive(Debug, Clone, Serialize)]
pub struct DbResultMessage {
    #[serde(rename = "type")]
    pub msg_type: &'static str, // always "result"
    pub call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

impl DbResultMessage {
    pub fn ok(call_id: String, data: serde_json::Value) -> Self {
        Self {
            msg_type: "result",
            call_id,
            op_id: None,
            data: Some(data),
            error: None,
        }
    }

    pub fn ok_with_op(call_id: String, op_id: Option<String>, data: serde_json::Value) -> Self {
        Self {
            msg_type: "result",
            call_id,
            op_id,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(call_id: String, code: &str, message: &str) -> Self {
        Self {
            msg_type: "result",
            call_id,
            op_id: None,
            data: None,
            error: Some(ErrorInfo {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }
    }

    pub fn err_with_op(
        call_id: String,
        op_id: Option<String>,
        code: &str,
        message: &str,
    ) -> Self {
        Self {
            msg_type: "result",
            call_id,
            op_id,
            data: None,
            error: Some(ErrorInfo {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// TypeScript → Rust messages
// ---------------------------------------------------------------------------

/// A message from the TypeScript handler back to Rust.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum TsMessage {
    /// DB operation request.
    #[serde(rename = "db")]
    Db(DbOpMessage),

    /// Stream a chunk to the HTTP client (SSE).
    #[serde(rename = "stream")]
    Stream(StreamChunkMessage),

    /// Schedule a function for later execution.
    #[serde(rename = "schedule")]
    Schedule(ScheduleMessage),

    /// Cancel a previously scheduled function.
    #[serde(rename = "cancel_schedule")]
    CancelSchedule(CancelScheduleMessage),

    /// Call another function (for actions calling queries/mutations).
    #[serde(rename = "run_fn")]
    RunFn(RunFnMessage),

    /// Function completed successfully.
    #[serde(rename = "return")]
    Return(ReturnMessage),

    /// Function failed with an error.
    #[serde(rename = "error")]
    Error(ErrorMessage),

    /// Initial handshake from the runtime: the list of functions it loaded.
    /// Sent once at startup before any other message.
    #[serde(rename = "ready")]
    Ready(ReadyMessage),
}

/// Handshake payload from the TS runtime.
#[derive(Debug, Clone, Deserialize)]
pub struct ReadyMessage {
    #[serde(default)]
    pub functions: Vec<crate::registry::FnDef>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A database operation request from TypeScript.
#[derive(Debug, Clone, Deserialize)]
pub struct DbOpMessage {
    pub call_id: String,
    /// Optional per-RPC id minted by the TS side. When present, the Rust
    /// reply echoes it back on `DbResultMessage.op_id` so the TS runtime
    /// can demux concurrent DB ops from a single handler (Promise.all).
    /// Legacy TS runtimes that don't send op_id keep the old behavior:
    /// only one in-flight RPC per call_id at a time.
    #[serde(default)]
    pub op_id: Option<String>,
    pub op: DbOp,
    pub entity: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    #[serde(default)]
    pub field: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub relation: Option<String>,
    #[serde(default)]
    pub target_id: Option<String>,
    /// Cursor pagination — `paginate` op only. Opaque id-after cursor.
    #[serde(default)]
    pub after: Option<String>,
    /// Cursor pagination — `paginate` op only. Requested page size.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Database operations available to TypeScript functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DbOp {
    Get,
    List,
    /// Cursor-paginated list. Uses `after` + `limit` on [`DbOpMessage`].
    /// Response shape: `{ page: Row[], nextCursor: string | null, isDone: bool }`.
    Paginate,
    Insert,
    Update,
    Delete,
    Lookup,
    Query,
    QueryGraph,
    Link,
    Unlink,
}

/// A stream chunk to forward to the HTTP client as SSE.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamChunkMessage {
    pub call_id: String,
    pub data: String,
    /// Optional event type for SSE (defaults to "message").
    #[serde(default)]
    pub event: Option<String>,
}

/// Schedule a function for future execution.
#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleMessage {
    pub call_id: String,
    pub fn_name: String,
    pub args: serde_json::Value,
    /// Run after this many milliseconds.
    #[serde(default)]
    pub delay_ms: Option<u64>,
    /// Run at this Unix timestamp (ms since epoch).
    #[serde(default)]
    pub run_at: Option<u64>,
}

/// Cancel a scheduled function.
#[derive(Debug, Clone, Deserialize)]
pub struct CancelScheduleMessage {
    pub call_id: String,
    pub schedule_id: String,
}

/// Call another function from within an action.
#[derive(Debug, Clone, Deserialize)]
pub struct RunFnMessage {
    pub call_id: String,
    pub fn_name: String,
    pub fn_type: FnType,
    pub args: serde_json::Value,
}

/// Function returned successfully.
#[derive(Debug, Clone, Deserialize)]
pub struct ReturnMessage {
    pub call_id: String,
    pub value: serde_json::Value,
}

/// Function failed.
#[derive(Debug, Clone, Deserialize)]
pub struct ErrorMessage {
    pub call_id: String,
    pub code: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

/// Function type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FnType {
    Query,
    Mutation,
    Action,
}

/// Auth context passed to function handlers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub is_admin: bool,
}

/// Error info in protocol messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_message_serializes() {
        let msg = CallMessage::new(
            "c1".into(),
            "placeBid".into(),
            FnType::Mutation,
            serde_json::json!({"lotId": "lot_1", "amount": 100}),
            AuthInfo {
                user_id: Some("user_1".into()),
                is_admin: false,
            },
        );
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"call\""));
        assert!(json.contains("\"fn_type\":\"mutation\""));
    }

    #[test]
    fn ts_message_deserializes_db_op() {
        let json = r#"{"type":"db","call_id":"c1","op":"get","entity":"Lot","id":"lot_1"}"#;
        let msg: TsMessage = serde_json::from_str(json).unwrap();
        match msg {
            TsMessage::Db(db) => {
                assert_eq!(db.call_id, "c1");
                assert_eq!(db.op, DbOp::Get);
                assert_eq!(db.entity, "Lot");
                assert_eq!(db.id.as_deref(), Some("lot_1"));
            }
            _ => panic!("expected Db message"),
        }
    }

    #[test]
    fn ts_message_deserializes_stream() {
        let json = r#"{"type":"stream","call_id":"c1","data":"hello"}"#;
        let msg: TsMessage = serde_json::from_str(json).unwrap();
        match msg {
            TsMessage::Stream(s) => {
                assert_eq!(s.data, "hello");
                assert!(s.event.is_none());
            }
            _ => panic!("expected Stream message"),
        }
    }

    #[test]
    fn ts_message_deserializes_return() {
        let json = r#"{"type":"return","call_id":"c1","value":{"ok":true}}"#;
        let msg: TsMessage = serde_json::from_str(json).unwrap();
        match msg {
            TsMessage::Return(r) => {
                assert_eq!(r.value, serde_json::json!({"ok": true}));
            }
            _ => panic!("expected Return message"),
        }
    }

    #[test]
    fn ts_message_deserializes_schedule() {
        let json = r#"{"type":"schedule","call_id":"c1","fn_name":"closeLot","args":{"lotId":"x"},"delay_ms":5000}"#;
        let msg: TsMessage = serde_json::from_str(json).unwrap();
        match msg {
            TsMessage::Schedule(s) => {
                assert_eq!(s.fn_name, "closeLot");
                assert_eq!(s.delay_ms, Some(5000));
            }
            _ => panic!("expected Schedule message"),
        }
    }

    #[test]
    fn db_result_ok() {
        let msg = DbResultMessage::ok("c1".into(), serde_json::json!({"id": "x"}));
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn db_result_err() {
        let msg = DbResultMessage::err("c1".into(), "NOT_FOUND", "not found");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"error\""));
        assert!(!json.contains("\"data\""));
    }
}
