//! Automatic instrumentation for function executions.
//!
//! Every function call produces a [`FnTrace`] with zero developer effort.
//! The Rust runtime timestamps each protocol message as it passes through,
//! recording DB operations, stream chunks, scheduled functions, and the outcome.
//! Detail limits bound retained memory. Omitted counts preserve operation totals.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::protocol::{DbOp, FnType};

// ---------------------------------------------------------------------------
// Trace types
// ---------------------------------------------------------------------------

/// A bounded trace of a single function execution.
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
    /// Total operations equal `ops.len() + ops_omitted`.
    pub ops_omitted: u64,
    pub stream_bytes: u64,
    pub stream_chunks: u32,
    pub schedules: Vec<ScheduleTrace>,
    /// Total schedules equal `schedules.len() + schedules_omitted`.
    pub schedules_omitted: u64,
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

// Bound both payload content and container overhead. Large values are omitted
// from traces; the function response keeps the complete value.
const MAX_TRACE_OPS: usize = 128;
const MAX_TRACE_SCHEDULES: usize = 128;
const MAX_TRACE_DETAIL_BYTES: usize = 32 * 1024;
const MAX_TRACE_ERROR_BYTES: usize = 4 * 1024;

const MAX_TRACE_VALUE_BYTES: usize = 4 * 1024;
const MAX_TRACE_VALUE_NODES: usize = 128;
const MAX_TRACE_VALUE_DEPTH: usize = 16;

fn trace_error_text(text: &str) -> String {
    if text.len() <= MAX_TRACE_ERROR_BYTES {
        return text.to_owned();
    }
    const SUFFIX: &str = " [truncated]";
    let mut end = MAX_TRACE_ERROR_BYTES - SUFFIX.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    // Allocate a bounded copy instead of retaining the original capacity.
    let mut bounded = String::with_capacity(MAX_TRACE_ERROR_BYTES);
    bounded.push_str(&text[..end]);
    bounded.push_str(SUFFIX);
    bounded
}

fn trace_value_fits(value: &serde_json::Value) -> bool {
    fn visit(
        value: &serde_json::Value,
        bytes: &mut usize,
        nodes: &mut usize,
        depth: usize,
    ) -> bool {
        if *nodes == 0 || depth > MAX_TRACE_VALUE_DEPTH {
            return false;
        }
        *nodes -= 1;
        let Some(left) = bytes.checked_sub(std::mem::size_of::<serde_json::Value>()) else {
            return false;
        };
        *bytes = left;
        match value {
            serde_json::Value::String(text) => {
                let Some(left) = bytes.checked_sub(text.len()) else {
                    return false;
                };
                *bytes = left;
                true
            }
            serde_json::Value::Array(items) => {
                items.len() <= *nodes
                    && items
                        .iter()
                        .all(|item| visit(item, bytes, nodes, depth + 1))
            }
            serde_json::Value::Object(fields) => {
                fields.len() <= *nodes
                    && fields.iter().all(|(key, value)| {
                        let Some(left) = bytes.checked_sub(key.len()) else {
                            return false;
                        };
                        *bytes = left;
                        visit(value, bytes, nodes, depth + 1)
                    })
            }
            _ => true,
        }
    }
    let mut bytes = MAX_TRACE_VALUE_BYTES;
    let mut nodes = MAX_TRACE_VALUE_NODES;
    visit(value, &mut bytes, &mut nodes, 0)
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
    ops_omitted: u64,
    schedules_omitted: u64,
    detail_bytes: usize,
    stream_bytes: u64,
    stream_chunks: u32,
    schedules: Vec<ScheduleTrace>,
}

impl TraceBuilder {
    pub fn new(call_id: String, fn_name: String, fn_type: FnType, user_id: Option<String>) -> Self {
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
            ops_omitted: 0,
            schedules_omitted: 0,
            detail_bytes: 0,
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
        let bytes = entity.len().saturating_add(id.map_or(0, str::len));
        if self.ops.len() >= MAX_TRACE_OPS || bytes > MAX_TRACE_DETAIL_BYTES - self.detail_bytes {
            self.ops_omitted = self.ops_omitted.saturating_add(1);
            return;
        }
        self.detail_bytes += bytes;
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
        if self.schedules.len() >= MAX_TRACE_SCHEDULES
            || fn_name.len() > MAX_TRACE_DETAIL_BYTES - self.detail_bytes
        {
            self.schedules_omitted = self.schedules_omitted.saturating_add(1);
            return;
        }
        self.detail_bytes += fn_name.len();
        self.schedules.push(ScheduleTrace {
            fn_name: fn_name.to_string(),
            delay_ms,
            run_at,
        });
    }

    /// Finalize the trace. Omit result values that exceed the trace budget.
    pub fn finish_ok(self, value: Option<serde_json::Value>) -> FnTrace {
        self.finish(FnOutcome::Ok {
            value: value.filter(trace_value_fits),
        })
    }

    /// Inspect the result before cloning it. Large responses need no trace copy.
    pub fn finish_ok_ref(self, value: &serde_json::Value) -> FnTrace {
        self.finish(FnOutcome::Ok {
            value: trace_value_fits(value).then(|| value.clone()),
        })
    }

    /// Finalize the trace with an error outcome.
    pub fn finish_error(self, code: String, message: String) -> FnTrace {
        self.finish_error_ref(&code, &message)
    }

    /// Copy only the stored error prefix. The caller keeps the full error.
    pub fn finish_error_ref(self, code: &str, message: &str) -> FnTrace {
        self.finish(FnOutcome::Error {
            code: trace_error_text(code),
            message: trace_error_text(message),
        })
    }

    /// Finalize the trace with a rollback outcome.
    pub fn finish_rolled_back(self, code: String, message: String) -> FnTrace {
        self.finish(FnOutcome::RolledBack {
            code: trace_error_text(&code),
            message: trace_error_text(&message),
        })
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
            ops_omitted: self.ops_omitted,
            schedules_omitted: self.schedules_omitted,
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
    buf: Vec<Arc<FnTrace>>,
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
        ring.count = ring.count.saturating_add(1);
        if cap == 0 {
            return;
        }
        let trace = Arc::new(trace);
        let evicted = if ring.buf.len() < cap {
            ring.buf.push(trace);
            None
        } else {
            let pos = ring.write_pos;
            Some(std::mem::replace(&mut ring.buf[pos], trace))
        };
        ring.write_pos = (ring.write_pos + 1) % cap;
        drop(ring);
        drop(evicted);
    }

    /// Query recent traces, newest first.
    pub fn recent(&self, limit: usize) -> Vec<FnTrace> {
        self.recent_matching(limit, |_| true)
    }

    /// Query traces filtered by function name, newest first.
    pub fn by_fn(&self, fn_name: &str, limit: usize) -> Vec<FnTrace> {
        self.recent_matching(limit, |trace| trace.fn_name == fn_name)
    }

    /// Query only error/rollback traces, newest first.
    pub fn errors(&self, limit: usize) -> Vec<FnTrace> {
        self.recent_matching(limit, |trace| {
            !matches!(trace.outcome, FnOutcome::Ok { .. })
        })
    }

    fn recent_matching(&self, limit: usize, matches: impl Fn(&FnTrace) -> bool) -> Vec<FnTrace> {
        self.shared_matching(limit, matches)
            .into_iter()
            .map(|trace| (*trace).clone())
            .collect()
    }

    /// Hold immutable entries while the pool selects its final result.
    pub(crate) fn recent_shared(&self, limit: usize) -> Vec<Arc<FnTrace>> {
        self.shared_matching(limit, |_| true)
    }

    fn shared_matching(
        &self,
        limit: usize,
        matches: impl Fn(&FnTrace) -> bool,
    ) -> Vec<Arc<FnTrace>> {
        let ring = self.traces.lock().unwrap();
        let len = ring.buf.len();
        let mut result = Vec::with_capacity(limit.min(len));
        if limit == 0 {
            return result;
        }
        for offset in 0..len {
            let index = (ring.write_pos + len - 1 - offset) % len;
            let trace = &ring.buf[index];
            if matches(trace) {
                result.push(trace.clone());
                if result.len() == limit {
                    break;
                }
            }
        }
        result
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
            ops_omitted: 0,
            schedules_omitted: 0,
            stream_bytes: 0,
            stream_chunks: 0,
            schedules: vec![],
        }
    }

    #[test]
    fn shared_trace_reads_retain_immutable_entries_after_eviction() {
        let log = TraceLog::new(1);
        log.push(make_trace("first", 1.0));
        let first = log.recent_shared(1);
        let second = log.recent_shared(1);
        assert!(Arc::ptr_eq(&first[0], &second[0]));
        log.push(make_trace("next", 2.0));
        assert_eq!(first[0].fn_name, "first");
        assert_eq!(log.recent(1)[0].fn_name, "next");
        let mut owned = log.recent(1);
        owned[0].fn_name = "changed".into();
        assert_eq!(log.recent(1)[0].fn_name, "next");
    }

    #[test]
    fn zero_capacity_counts_calls_without_retaining_traces() {
        let log = TraceLog::new(0);
        log.push(make_trace("a", 1.0));
        log.push(make_trace("b", 2.0));
        assert_eq!(log.total_count(), 2);
        assert!(log.is_empty());
        assert!(log.recent(10).is_empty());
        assert!(log.by_fn("a", 10).is_empty());
        assert!(log.errors(10).is_empty());
    }

    #[test]
    fn trace_detail_counts_and_shared_byte_budget_are_bounded() {
        let mut builder = TraceBuilder::new("c".into(), "bulk".into(), FnType::Mutation, None);
        for _ in 0..10_000 {
            builder.record_op(DbOp::Get, "E", Some("id"), Duration::ZERO, Some(1), true);
            builder.record_schedule("later", None, None);
        }
        let trace = builder.finish_ok(None);
        assert_eq!(trace.ops.len(), MAX_TRACE_OPS);
        assert_eq!(trace.schedules.len(), MAX_TRACE_SCHEDULES);
        assert_eq!(trace.ops.len() as u64 + trace.ops_omitted, 10_000);
        assert_eq!(
            trace.schedules.len() as u64 + trace.schedules_omitted,
            10_000
        );
        let serialized = serde_json::to_value(&trace).unwrap();
        assert_eq!(serialized["ops_omitted"], 10_000 - MAX_TRACE_OPS);

        let mut builder = TraceBuilder::new("c".into(), "bulk".into(), FnType::Mutation, None);
        let huge_id = "x".repeat(MAX_TRACE_DETAIL_BYTES + 1);
        builder.record_op(DbOp::Get, "E", Some(&huge_id), Duration::ZERO, None, false);
        // An oversized detail must not consume the budget for later entries.
        let id = "x".repeat(MAX_TRACE_DETAIL_BYTES - 1);
        builder.record_op(DbOp::Get, "E", Some(&id), Duration::ZERO, None, true);
        builder.record_schedule("later", None, None);
        let trace = builder.finish_ok(None);
        assert_eq!(trace.ops.len(), 1);
        assert_eq!(trace.ops_omitted, 1);
        assert_eq!(trace.schedules_omitted, 1);
        assert!(trace.schedules.is_empty());
        assert_eq!(
            trace.ops[0].entity.len() + trace.ops[0].id.as_ref().unwrap().len(),
            MAX_TRACE_DETAIL_BYTES
        );
    }

    #[test]
    fn trace_error_text_is_bounded_at_utf8_boundaries() {
        let text = "文😀".repeat(100_000);
        let builder = || TraceBuilder::new("c".into(), "error".into(), FnType::Action, None);
        for trace in [
            builder().finish_error_ref(&text, &text),
            builder().finish_rolled_back(text.clone(), text.clone()),
        ] {
            let (code, message) = match trace.outcome {
                FnOutcome::Error { code, message } | FnOutcome::RolledBack { code, message } => {
                    (code, message)
                }
                _ => panic!("expected error"),
            };
            for value in [code, message] {
                assert!(value.len() <= MAX_TRACE_ERROR_BYTES);
                assert!(value.capacity() <= MAX_TRACE_ERROR_BYTES);
                assert!(value.ends_with(" [truncated]"));
                assert!(text.starts_with(value.trim_end_matches(" [truncated]")));
            }
        }
        assert_eq!(text.len(), 700_000);
        assert_eq!(trace_error_text("unchanged"), "unchanged");
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

    fn success_trace(value: &serde_json::Value) -> FnTrace {
        TraceBuilder::new("c".into(), "query".into(), FnType::Query, None).finish_ok_ref(value)
    }

    #[test]
    fn trace_values_are_bounded_without_changing_the_response() {
        let small = serde_json::json!({"accepted": true});
        assert_eq!(
            serde_json::to_value(success_trace(&small)).unwrap()["outcome"]["value"],
            small
        );
        let large = serde_json::json!({"body": "x".repeat(1024 * 1024)});
        let trace = success_trace(&large);
        assert!(matches!(trace.outcome, FnOutcome::Ok { value: None }));
        assert_eq!(large["body"].as_str().unwrap().len(), 1024 * 1024);
        let owned = TraceBuilder::new("c".into(), "query".into(), FnType::Query, None)
            .finish_ok(Some(large));
        assert!(matches!(owned.outcome, FnOutcome::Ok { value: None }));
    }

    #[test]
    fn trace_budget_counts_containers_keys_and_depth() {
        let many = serde_json::json!(vec![serde_json::Value::Null; MAX_TRACE_VALUE_NODES + 1]);
        let long_key = serde_json::json!({"k".repeat(MAX_TRACE_VALUE_BYTES): true});
        let mut nested = serde_json::Value::Null;
        for _ in 0..MAX_TRACE_VALUE_DEPTH + 1 {
            nested = serde_json::json!([nested]);
        }
        for value in [many, long_key, nested] {
            assert!(matches!(
                success_trace(&value).outcome,
                FnOutcome::Ok { value: None }
            ));
        }
    }

    #[test]
    fn trace_ring_retains_no_large_result_payloads() {
        let log = TraceLog::new(1000);
        let large = serde_json::json!("x".repeat(1024 * 1024));
        for _ in 0..1000 {
            log.push(success_trace(&large));
        }
        let traces = log.recent(1000);
        assert_eq!(traces.len(), 1000);
        assert!(traces
            .iter()
            .all(|trace| matches!(trace.outcome, FnOutcome::Ok { value: None })));
        assert!(serde_json::to_vec(&traces).unwrap().len() < 1024 * 1024);
    }

    #[test]
    fn filtered_trace_reads_keep_order_and_limits_after_wraparound() {
        let log = TraceLog::new(3);
        for (name, duration) in [
            ("match", 0.0),
            ("match", 1.0),
            ("skip", 2.0),
            ("match", 3.0),
        ] {
            log.push(make_trace(name, duration));
        }
        let result = log.by_fn("match", 1);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].duration_ms, 3.0);
        assert_eq!(log.by_fn("match", 9).len(), 2);
        assert!(log.by_fn("match", 0).is_empty());
        assert!(TraceLog::new(3).recent(1).is_empty());
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
