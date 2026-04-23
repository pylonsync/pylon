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

    fn get_by_id(
        &self,
        entity: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, DataError>;

    fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, DataError>;

    fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DataError>;

    fn update(
        &self,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
    ) -> Result<bool, DataError>;

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

    fn unlink(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
    ) -> Result<bool, DataError>;

    fn query_filtered(
        &self,
        entity: &str,
        filter: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, DataError>;

    fn query_graph(
        &self,
        query: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError>;

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
