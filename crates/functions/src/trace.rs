//! Automatic instrumentation for function executions.
//!
//! Every function call produces a [`FnTrace`] with zero developer effort.
//! The Rust runtime timestamps each protocol message as it passes through,
//! building a complete trace of all DB operations, stream chunks, scheduled
//! functions, and the final outcome.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::protocol::{DbOp, FnType};

// ---------------------------------------------------------------------------
// Trace types
// ---------------------------------------------------------------------------

/// A complete trace of a single function execution.
#[derive(Debug, Clone, Serialize)]
pub struct FnTrace {
    pub call_id: String,
    pub fn_name: String,
    pub fn_type: FnType,
    pub user_id: Option<String>,
    pub started_at: u64,
    pub duration_ms: f64,
    pub outcome: FnOutcome,
    pub ops: Vec<OpTrace>,
    pub stream_bytes: u64,
    pub stream_chunks: u32,
    pub schedules: Vec<ScheduleTrace>,
}

/// How the function completed.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status")]
pub enum FnOutcome {
    #[serde(rename = "ok")]
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<serde_json::Value>,
    },
    #[serde(rename = "error")]
    Error { code: String, message: String },
    #[serde(rename = "rolled_back")]
    RolledBack { code: String, message: String },
}

/// Trace of a single DB operation within a function.
#[derive(Debug, Clone, Serialize)]
pub struct OpTrace {
    pub op: DbOp,
    pub entity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub duration_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_count: Option<usize>,
    pub ok: bool,
}

/// Trace of a scheduled function call.
#[derive(Debug, Clone, Serialize)]
pub struct ScheduleTrace {
    pub fn_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_at: Option<u64>,
}

// ---------------------------------------------------------------------------
// Trace builder — accumulates during execution
// ---------------------------------------------------------------------------

/// Accumulates trace data during a function execution.
///
/// Created at the start of each function call. Each protocol message
/// updates the builder. When the function completes, `finish()` produces
/// the final [`FnTrace`].
pub struct TraceBuilder {
    call_id: String,
    fn_name: String,
    fn_type: FnType,
    pub(crate) user_id: Option<String>,
    /// Active tenant at call time. Threaded through so nested calls
    /// (action → mutation) can inherit it when row-level policies gate
    /// every write the action emits.
    pub(crate) tenant_id: Option<String>,
    started_at: u64,
    start_instant: Instant,
    ops: Vec<OpTrace>,
    stream_bytes: u64,
    stream_chunks: u32,
    schedules: Vec<ScheduleTrace>,
}

impl TraceBuilder {
    pub fn new(
        call_id: String,
        fn_name: String,
        fn_type: FnType,
        user_id: Option<String>,
    ) -> Self {
        Self::new_with_tenant(call_id, fn_name, fn_type, user_id, None)
    }

    pub fn new_with_tenant(
        call_id: String,
        fn_name: String,
        fn_type: FnType,
        user_id: Option<String>,
        tenant_id: Option<String>,
    ) -> Self {
        let now_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            call_id,
            fn_name,
            fn_type,
            user_id,
            tenant_id,
            started_at: now_epoch,
            start_instant: Instant::now(),
            ops: Vec::new(),
            stream_bytes: 0,
            stream_chunks: 0,
            schedules: Vec::new(),
        }
    }

    /// Tenant at call time. Used by the nested-call path in the runner to
    /// carry tenant id down to helper mutations an action invokes.
    pub fn tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }

    /// Record a completed DB operation.
    pub fn record_op(
        &mut self,
        op: DbOp,
        entity: &str,
        id: Option<&str>,
        duration: Duration,
        row_count: Option<usize>,
        ok: bool,
    ) {
        self.ops.push(OpTrace {
            op,
            entity: entity.to_string(),
            id: id.map(|s| s.to_string()),
            duration_ms: duration.as_secs_f64() * 1000.0,
            row_count,
            ok,
        });
    }

    /// Record a stream chunk sent to the client.
    pub fn record_stream_chunk(&mut self, bytes: usize) {
        self.stream_bytes += bytes as u64;
        self.stream_chunks += 1;
    }

    /// Record a scheduled function.
    pub fn record_schedule(&mut self, fn_name: &str, delay_ms: Option<u64>, run_at: Option<u64>) {
        self.schedules.push(ScheduleTrace {
            fn_name: fn_name.to_string(),
            delay_ms,
            run_at,
        });
    }

    /// Finalize the trace with a successful outcome.
    pub fn finish_ok(self, value: Option<serde_json::Value>) -> FnTrace {
        self.finish(FnOutcome::Ok { value })
    }

    /// Finalize the trace with an error outcome.
    pub fn finish_error(self, code: String, message: String) -> FnTrace {
        self.finish(FnOutcome::Error { code, message })
    }

    /// Finalize the trace with a rollback outcome.
    pub fn finish_rolled_back(self, code: String, message: String) -> FnTrace {
        self.finish(FnOutcome::RolledBack { code, message })
    }

    fn finish(self, outcome: FnOutcome) -> FnTrace {
        FnTrace {
            call_id: self.call_id,
            fn_name: self.fn_name,
            fn_type: self.fn_type,
            user_id: self.user_id,
            started_at: self.started_at,
            duration_ms: self.start_instant.elapsed().as_secs_f64() * 1000.0,
            outcome,
            ops: self.ops,
            stream_bytes: self.stream_bytes,
            stream_chunks: self.stream_chunks,
            schedules: self.schedules,
        }
    }
}

// ---------------------------------------------------------------------------
// Trace log — bounded ring buffer of recent traces
// ---------------------------------------------------------------------------

/// A bounded ring buffer of recent function traces.
///
/// Thread-safe. Stores the most recent `capacity` traces. Oldest entries
/// are evicted when the buffer is full.
pub struct TraceLog {
    traces: std::sync::Mutex<TraceRing>,
}

struct TraceRing {
    buf: Vec<FnTrace>,
    capacity: usize,
    write_pos: usize,
    count: usize,
}

impl TraceLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            traces: std::sync::Mutex::new(TraceRing {
                buf: Vec::with_capacity(capacity),
                capacity,
                write_pos: 0,
                count: 0,
            }),
        }
    }

    /// Record a completed trace.
    pub fn push(&self, trace: FnTrace) {
        let mut ring = self.traces.lock().unwrap();
        let cap = ring.capacity;
        if ring.buf.len() < cap {
            ring.buf.push(trace);
        } else {
            let pos = ring.write_pos;
            ring.buf[pos] = trace;
        }
        ring.write_pos = (ring.write_pos + 1) % cap;
        ring.count += 1;
    }

    /// Query recent traces, newest first.
    pub fn recent(&self, limit: usize) -> Vec<FnTrace> {
        let ring = self.traces.lock().unwrap();
        let len = ring.buf.len();
        if len == 0 {
            return vec![];
        }

        let take = limit.min(len);
        let mut result = Vec::with_capacity(take);

        // Walk backwards from the most recent write position.
        let start = if ring.write_pos == 0 { len - 1 } else { ring.write_pos - 1 };
        let mut i = start;
        for _ in 0..take {
            result.push(ring.buf[i].clone());
            if i == 0 {
                i = len - 1;
            } else {
                i -= 1;
            }
        }
        result
    }

    /// Query traces filtered by function name, newest first.
    pub fn by_fn(&self, fn_name: &str, limit: usize) -> Vec<FnTrace> {
        self.recent(self.len())
            .into_iter()
            .filter(|t| t.fn_name == fn_name)
            .take(limit)
            .collect()
    }

    /// Query only error/rollback traces, newest first.
    pub fn errors(&self, limit: usize) -> Vec<FnTrace> {
        self.recent(self.len())
            .into_iter()
            .filter(|t| !matches!(t.outcome, FnOutcome::Ok { .. }))
            .take(limit)
            .collect()
    }

    /// Total traces recorded (including evicted).
    pub fn total_count(&self) -> usize {
        self.traces.lock().unwrap().count
    }

    /// Current buffer size.
    pub fn len(&self) -> usize {
        self.traces.lock().unwrap().buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_trace(name: &str, duration_ms: f64) -> FnTrace {
        FnTrace {
            call_id: format!("c_{name}"),
            fn_name: name.to_string(),
            fn_type: FnType::Mutation,
            user_id: Some("user_1".to_string()),
            started_at: 1000,
            duration_ms,
            outcome: FnOutcome::Ok { value: None },
            ops: vec![],
            stream_bytes: 0,
            stream_chunks: 0,
            schedules: vec![],
        }
    }

    #[test]
    fn trace_builder_records_ops() {
        let mut builder = TraceBuilder::new(
            "c1".into(),
            "placeBid".into(),
            FnType::Mutation,
            Some("user_1".into()),
        );

        builder.record_op(
            DbOp::Get,
            "Lot",
            Some("lot_1"),
            Duration::from_micros(100),
            Some(1),
            true,
        );
        builder.record_op(
            DbOp::Insert,
            "Bid",
            None,
            Duration::from_micros(150),
            None,
            true,
        );
        builder.record_stream_chunk(42);
        builder.record_stream_chunk(18);
        builder.record_schedule("closeLot", Some(5000), None);

        let trace = builder.finish_ok(Some(serde_json::json!({"accepted": true})));

        assert_eq!(trace.fn_name, "placeBid");
        assert_eq!(trace.ops.len(), 2);
        assert_eq!(trace.stream_bytes, 60);
        assert_eq!(trace.stream_chunks, 2);
        assert_eq!(trace.schedules.len(), 1);
    }

    #[test]
    fn trace_log_ring_buffer() {
        let log = TraceLog::new(3);

        log.push(make_trace("a", 1.0));
        log.push(make_trace("b", 2.0));
        log.push(make_trace("c", 3.0));
        log.push(make_trace("d", 4.0)); // evicts "a"

        assert_eq!(log.len(), 3);
        assert_eq!(log.total_count(), 4);

        let recent = log.recent(10);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].fn_name, "d"); // newest first
        assert_eq!(recent[1].fn_name, "c");
        assert_eq!(recent[2].fn_name, "b");
    }

    #[test]
    fn trace_log_by_fn() {
        let log = TraceLog::new(100);
        log.push(make_trace("placeBid", 1.0));
        log.push(make_trace("getLots", 0.5));
        log.push(make_trace("placeBid", 1.2));

        let bids = log.by_fn("placeBid", 10);
        assert_eq!(bids.len(), 2);
    }

    #[test]
    fn trace_log_errors() {
        let log = TraceLog::new(100);
        log.push(make_trace("a", 1.0));

        let mut err_trace = make_trace("b", 2.0);
        err_trace.outcome = FnOutcome::Error {
            code: "BID_TOO_LOW".into(),
            message: "too low".into(),
        };
        log.push(err_trace);

        let errors = log.errors(10);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].fn_name, "b");
    }

    #[test]
    fn trace_serializes() {
        let trace = make_trace("test", 1.5);
        let json = serde_json::to_string(&trace).unwrap();
        assert!(json.contains("\"fn_name\":\"test\""));
        assert!(json.contains("\"status\":\"ok\""));
    }
}
