use std::fmt;

// ---------------------------------------------------------------------------
// HttpMethod — platform-agnostic HTTP verb
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Options,
    Head,
}

impl HttpMethod {
    /// Parse an HTTP method string. Returns `None` for unrecognized methods.
    pub fn try_parse(s: &str) -> Option<Self> {
        match s {
            "GET" | "get" => Some(Self::Get),
            "POST" | "post" => Some(Self::Post),
            "PUT" | "put" => Some(Self::Put),
            "PATCH" | "patch" => Some(Self::Patch),
            "DELETE" | "delete" => Some(Self::Delete),
            "OPTIONS" | "options" => Some(Self::Options),
            "HEAD" | "head" => Some(Self::Head),
            _ => None,
        }
    }

    /// Parse an HTTP method string, falling back to `Get` for unrecognized methods.
    /// Prefer `try_parse` to detect malformed inputs; this remains for compatibility.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        Self::try_parse(s).unwrap_or(Self::Get)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Options => "OPTIONS",
            Self::Head => "HEAD",
        }
    }

    /// True for methods that never have a request body.
    pub fn is_bodyless(&self) -> bool {
        matches!(self, Self::Get | Self::Head | Self::Options | Self::Delete)
    }
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// DataError — platform-agnostic error from data operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DataError {
    pub code: String,
    pub message: String,
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for DataError {}

// ---------------------------------------------------------------------------
// DataStore — platform-agnostic data access trait
// ---------------------------------------------------------------------------

/// Platform-agnostic data store trait.
///
/// Implemented by `Runtime` (SQLite, self-hosted) and `D1DataStore` (Workers).
/// All methods are synchronous to keep the trait `Send + Sync` and simple;
/// Workers adapters can use `block_on` or similar bridging.
pub trait DataStore: Send + Sync {
    fn manifest(&self) -> &pylon_kernel::AppManifest;

    fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, DataError>;

    fn get_by_id(&self, entity: &str, id: &str) -> Result<Option<serde_json::Value>, DataError>;

    fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, DataError>;

    fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DataError>;

    fn update(&self, entity: &str, id: &str, data: &serde_json::Value) -> Result<bool, DataError>;

    fn delete(&self, entity: &str, id: &str) -> Result<bool, DataError>;

    fn lookup(
        &self,
        entity: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<serde_json::Value>, DataError>;

    fn link(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, DataError>;

    fn unlink(&self, entity: &str, id: &str, relation: &str) -> Result<bool, DataError>;

    fn query_filtered(
        &self,
        entity: &str,
        filter: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, DataError>;

    fn query_graph(&self, query: &serde_json::Value) -> Result<serde_json::Value, DataError>;

    /// Run an aggregation query.
    ///
    /// Spec shape (same vocabulary in the HTTP body):
    /// ```json
    /// {
    ///   "count": "*",
    ///   "sum": ["amount"],
    ///   "avg": ["price"],
    ///   "min": ["createdAt"],
    ///   "max": ["createdAt"],
    ///   "groupBy": ["status"],
    ///   "where": { ...standard filter... }
    /// }
    /// ```
    /// Returns `{rows: [{count, sum_amount, ...}]}`.
    /// Default implementation returns `NOT_SUPPORTED`; Runtime overrides it.
    fn aggregate(
        &self,
        _entity: &str,
        _spec: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        Err(DataError {
            code: "NOT_SUPPORTED".into(),
            message: "aggregate() is not implemented by this backend".into(),
        })
    }

    /// Execute transactional operations. Each element is a JSON object with
    /// `op` ("insert"/"update"/"delete"), `entity`, and optionally `id`/`data`.
    ///
    /// Returns per-operation results. The implementation decides whether to
    /// use real SQL transactions (Runtime) or sequential execution (D1).
    fn transact(
        &self,
        ops: &[serde_json::Value],
    ) -> Result<(bool, Vec<serde_json::Value>), DataError>;

    /// Run a faceted full-text search against a searchable entity. `query`
    /// is a JSON object with the keys defined by `SearchQuery` in
    /// `pylon_storage::search`; returns a JSON object shaped like
    /// `SearchResult` (`{ hits, facetCounts, total, tookMs }`).
    ///
    /// Default impl returns `NOT_SUPPORTED`; Runtime overrides it. The
    /// value is raw JSON (not a typed struct) so backends without a
    /// dependency on pylon-storage can still compile.
    fn search(
        &self,
        _entity: &str,
        _query: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        Err(DataError {
            code: "NOT_SUPPORTED".into(),
            message: "search() is not implemented by this backend".into(),
        })
    }

    /// Return the binary CRDT snapshot for a row, used by the router
    /// to ship a binary update over WebSocket after every successful
    /// write.
    ///
    /// Return value semantics:
    /// - `Ok(Some(bytes))` — entity is CRDT-mode and bytes are the
    ///   current Loro snapshot for the row.
    /// - `Ok(None)` — **either** the entity is `crdt: false` (LWW
    ///   opt-out) **or** this backend doesn't support CRDT mode at
    ///   all. Callers MUST treat both cases identically: skip the
    ///   binary broadcast and rely on the JSON change event for
    ///   client invalidation. The conflation is intentional — every
    ///   caller today does the same thing in both cases, and a
    ///   richer enum (NotCrdtMode / NotSupported) would be carried
    ///   through every layer for no behavioral payoff.
    /// - `Err(_)` — entity is CRDT-mode but the snapshot fetch
    ///   itself failed (schema lookup, sidecar read, decode). Log
    ///   and continue; the JSON change event already covers the
    ///   correctness path.
    ///
    /// Default impl returns `Ok(None)` so backends that don't support
    /// CRDT mode (e.g. the Workers D1 store at time of writing)
    /// compile without ceremony. Per the Ok(None) semantics above,
    /// this is correct behavior, not a stub.
    fn crdt_snapshot(&self, _entity: &str, _row_id: &str) -> Result<Option<Vec<u8>>, DataError> {
        Ok(None)
    }

    /// Apply a binary CRDT update from a client to the row's LoroDoc,
    /// project the new state into the SQLite materialized view, and
    /// return the post-merge snapshot bytes (so the caller can
    /// broadcast them to OTHER subscribed clients).
    ///
    /// `update` is opaque Loro bytes — either a snapshot or an
    /// incremental delta. Loro's import contract accepts both shapes,
    /// so the store doesn't need to know which the client sent.
    ///
    /// Errors:
    /// - `ENTITY_NOT_FOUND` — unknown entity in the manifest.
    /// - `NOT_SUPPORTED` — entity is `crdt: false` (LWW opt-out) or
    ///   the backend doesn't implement CRDT mode.
    /// - `CRDT_DECODE_FAILED` — bytes weren't a valid Loro update.
    /// - Storage failures from the underlying SQLite write.
    ///
    /// Default impl returns `NOT_SUPPORTED` so backends without CRDT
    /// support compile cleanly.
    fn crdt_apply_update(
        &self,
        _entity: &str,
        _row_id: &str,
        _update: &[u8],
    ) -> Result<Vec<u8>, DataError> {
        Err(DataError {
            code: "NOT_SUPPORTED".into(),
            message: "crdt_apply_update() is not implemented by this backend".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_method_roundtrip() {
        assert_eq!(HttpMethod::from_str("GET"), HttpMethod::Get);
        assert_eq!(HttpMethod::from_str("post"), HttpMethod::Post);
        assert_eq!(HttpMethod::from_str("DELETE"), HttpMethod::Delete);
        assert_eq!(HttpMethod::Get.as_str(), "GET");
    }

    #[test]
    fn data_error_display() {
        let e = DataError {
            code: "TEST".into(),
            message: "fail".into(),
        };
        assert_eq!(format!("{e}"), "[TEST] fail");
    }
}
