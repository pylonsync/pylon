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

    /// Per-operation privilege signal from the caller. The function
    /// runner calls this before every db op with the CURRENT admin
    /// state, so a mid-call `ctx.auth.elevate({ admin: true })` reaches
    /// wrappers that captured their auth context at call entry (the
    /// plugin-chain wrapper does — without this, elevate affected the
    /// policy gate but the owner/tenant stamp plugins still saw the
    /// pre-elevation caller and rejected on-behalf-of writes). Default
    /// no-op: plain stores don't gate on auth.
    fn set_op_admin(&self, _admin: bool) {}

    fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, DataError>;

    fn get_by_id(&self, entity: &str, id: &str) -> Result<Option<serde_json::Value>, DataError>;

    fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, DataError>;

    fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, DataError>;

    /// Rows in DESCENDING id order — newest first — starting strictly below
    /// `before` when given. The mirror of [`DataStore::list_after`].
    ///
    /// Exists for `sync_limit`. Every other scan here walks ids ASCENDING, so
    /// truncating one gave the replica the OLDEST rows; the cap is aimed at
    /// time-series tables, where the head is precisely what nobody wants.
    ///
    /// It takes a cursor rather than just a count because the cap means "at
    /// most N rows THIS CALLER can see", and visibility is decided per row by
    /// the read policy. A single fixed-size fetch would take the newest N rows
    /// across ALL tenants and then filter — so on a busy multi-tenant table a
    /// small cap could hand a quiet tenant nothing at all. Paging backwards
    /// lets the caller keep reading until it has N *visible* rows.
    ///
    /// `Ok(None)` means "this store can't scan backwards"; the caller falls
    /// back to the ascending path rather than failing. Default-implemented so
    /// the many test stubs implementing `DataStore` don't all have to care.
    fn list_last(
        &self,
        _entity: &str,
        _before: Option<&str>,
        _limit: usize,
    ) -> Result<Option<Vec<serde_json::Value>>, DataError> {
        Ok(None)
    }

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

    /// Acquire a transaction-scoped advisory lock on `key`.
    ///
    /// Used by application code to close TOCTOU windows around
    /// quota/uniqueness checks ("count then insert" patterns where two
    /// concurrent transactions both pass the count then both insert,
    /// blowing past the cap). When called from a mutation handler, the
    /// lock is held for the duration of the handler's transaction and
    /// released automatically on commit or rollback.
    ///
    /// Backend semantics:
    /// - Postgres: `SELECT pg_advisory_xact_lock(hash(key))`. Two
    ///   concurrent mutations holding the same key serialize.
    /// - SQLite: noop. SQLite already serializes writers via the
    ///   per-connection lock, so a "count + insert" inside a single
    ///   mutation transaction is already safe against parallel
    ///   handlers — the second handler waits for the first to commit
    ///   before seeing any rows. The trait method is a noop here so
    ///   application code can use the same primitive across both
    ///   backends without conditional logic.
    /// - D1 / Workers / read-only stores: returns NOT_SUPPORTED so
    ///   callers can fail loudly if they assumed the gate was on.
    ///
    /// `key` is hashed into a stable integer; pick a string that
    /// uniquely identifies the resource being gated (e.g.
    /// `format!("org_count:{user_id}")`).
    fn advisory_lock(&self, _key: &str) -> Result<(), DataError> {
        // Default noop. SQLite hits this path; PG overrides.
        Ok(())
    }

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

    /// Run an exact k-NN vector search over a `vector(dims)` field.
    /// `query` is a JSON object shaped like `VectorQuery` in
    /// `pylon_storage::vector` (`{ field, vector, limit?, metric?,
    /// filter? }`); returns JSON shaped like `VectorSearchResult`
    /// (`{ hits: [{id, score, doc}], tookMs }`), hits best-first.
    ///
    /// Same raw-JSON contract as `search()` — backends without a
    /// pylon-storage dependency compile against the default.
    fn vector_search(
        &self,
        _entity: &str,
        _query: &serde_json::Value,
    ) -> Result<serde_json::Value, DataError> {
        Err(DataError {
            code: "NOT_SUPPORTED".into(),
            message: "vector_search() is not implemented by this backend".into(),
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

    /// Return the row's current Loro version vector as opaque bytes
    /// (Loro's own `VersionVector::encode` format). Pylon's WS broadcast
    /// path uses this to remember "what state did all subscribers last
    /// receive?" so the next write can ship an incremental delta
    /// against that vector instead of a full snapshot.
    ///
    /// Default impl returns Ok(None) — the WS broadcast falls back to
    /// the snapshot path. Backends with real LoroDoc access override.
    fn crdt_vv(&self, _entity: &str, _row_id: &str) -> Result<Option<Vec<u8>>, DataError> {
        Ok(None)
    }

    /// Return the incremental Loro update bytes that advance a peer
    /// at version `since` to the row's current state. `since` is bytes
    /// previously returned from `crdt_vv` (or from a peer's own
    /// `oplog_vv().encode()`).
    ///
    /// Returns Ok(None) when:
    ///   - the entity isn't CRDT
    ///   - the row doesn't exist
    ///   - the backend doesn't support deltas (override required)
    ///
    /// The bytes are suitable for direct `LoroDoc::import(...)` on the
    /// receiving side and are typically MUCH smaller than a snapshot
    /// for incremental edits (a single-field update is ~50-150 bytes
    /// vs ~200-500 bytes for the compacted snapshot).
    fn crdt_update_since(
        &self,
        _entity: &str,
        _row_id: &str,
        _since: &[u8],
    ) -> Result<Option<Vec<u8>>, DataError> {
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
