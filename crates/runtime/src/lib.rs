pub mod account_backend;
pub mod api_key_backend;
pub mod audit_backend;
pub mod cache_handlers;
pub mod cache_server;
pub mod change_log_persister;
pub mod change_log_store;
pub mod config;
pub mod connections;
pub mod cron;
pub mod datastore;
pub mod dev_diagnostics;
pub mod encryption;
pub mod file_urls;
pub mod frontend;
pub mod image_optim;
pub mod ip_limit;
pub mod job_store;
pub mod jobs;
pub mod leader;
pub mod llm;
pub mod log;
pub mod log_ring;
pub mod loro_store;
pub mod magic_code_backend;
pub mod markdown;
pub mod metrics;
pub mod oauth_backend;
pub mod openapi;
pub mod org_sso_backend;
pub mod pg_boot_guard;
pub mod pg_loro_store;
pub mod presence;
pub mod pubsub;
pub mod rate_limit;
pub mod reactive;
pub mod resp;
pub mod resp_server;
pub mod rooms;
pub mod saml_backend;
pub mod scheduler;
pub mod seq_allocator;
pub mod server;
pub mod session_backend;
pub mod shard_ws;
pub mod sse;
pub mod ssr_cache;
pub mod stream_hub;
pub mod sync_relay;
pub mod tinybird_logger;
pub mod tls;
pub mod trusted_device_backend;
pub mod verification_backend;
pub mod workflow_store;
pub mod workflows;
pub mod ws;

/// End-to-end lifecycle harness — boots the real runtime + change-log
/// wiring against a file-backed SQLite DB and drives it through
/// restart / reload / reconnect / sleep transitions. Test-only.
#[cfg(test)]
mod lifecycle_scenario;

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// Bind a TCP listener that accepts BOTH IPv4 and IPv6 connections.
///
/// macOS resolves `localhost` to `::1` first (IPv6). A v4-only listener
/// at `0.0.0.0:port` would silently refuse those connects — the client
/// retries the next address (`127.0.0.1`) but applications that don't
/// retry (e.g. the Yapless Mac app's WebSocket connect) just see
/// "connection refused" with no useful diagnostic.
///
/// Pattern: try `[::]:port` first (dual-stack, the kernel maps v4
/// connections via IPv4-mapped IPv6 addresses on Linux + macOS), fall
/// back to `0.0.0.0:port` if v6 is unavailable. The fallback exists
/// because some sandboxed test environments and old Linux kernels
/// disable IPv6 entirely; we don't want to refuse to start there.
pub fn bind_dual_stack_tcp(port: u16) -> Result<TcpListener, std::io::Error> {
    let v6 = format!("[::]:{port}");
    match TcpListener::bind(&v6) {
        Ok(l) => Ok(l),
        Err(_) => TcpListener::bind(format!("0.0.0.0:{port}")),
    }
}

/// Accept one connection without tripping libstd's `sockaddr` assertion.
///
/// On macOS a dual-stack `[::]` listener (see [`bind_dual_stack_tcp`]) can
/// have `accept()` / `getpeername()` hand back an `AF_INET6` address whose
/// length is shorter than `sockaddr_in6` — a peer that RSTs between `listen`
/// and `accept`, and some v4-mapped cases. libstd's `sockaddr_to_addr` does
/// `assert!(len >= size_of::<sockaddr_in6>())`, which PANICS (not errors) and
/// kills the accept thread — the crash a fresh `pylon dev` hit on macOS.
/// `TcpListener::incoming()` builds that `SocketAddr` even though it discards
/// it, so the panic happens at accept time, before any `peer_addr()` guard.
///
/// We sidestep it: accept with a NULL address pointer (the kernel writes no
/// peer address, so there is nothing to parse or assert), then read the peer
/// IP via `getpeername` and decode it ourselves with an explicit length check
/// — an address we can't make sense of yields `None` instead of a panic.
///
/// Returns the accepted (blocking) stream plus a best-effort peer IP. Callers
/// enforcing a per-IP cap should bucket a `None` peer under a sentinel.
pub fn accept_tcp(
    listener: &TcpListener,
) -> Result<(std::net::TcpStream, Option<std::net::IpAddr>), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::io::{AsRawFd, FromRawFd};
        // SAFETY: `accept` with null addr/len pointers is explicitly valid —
        // it tells the kernel not to write a peer address. The returned fd is
        // owned by us; we hand it straight to `TcpStream::from_raw_fd`.
        let fd = unsafe {
            libc::accept(
                listener.as_raw_fd(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let stream = unsafe { std::net::TcpStream::from_raw_fd(fd) };
        Ok((stream, peer_ip_unix(fd)))
    }
    #[cfg(not(unix))]
    {
        // Other platforms (Windows): libstd's accept doesn't carry the Unix
        // sockaddr assertion, so the plain path is panic-safe.
        let (stream, addr) = listener.accept()?;
        Ok((stream, Some(addr.ip())))
    }
}

/// Best-effort peer IP for an accepted fd, decoded with a strict length
/// check so a truncated address (the very thing that panics libstd) returns
/// `None` instead of reading past what the kernel wrote.
#[cfg(unix)]
fn peer_ip_unix(fd: std::os::unix::io::RawFd) -> Option<std::net::IpAddr> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    // SAFETY: `storage` is a zeroed sockaddr_storage with `len` set to its
    // size; getpeername writes at most `len` bytes and updates `len`.
    let rc =
        unsafe { libc::getpeername(fd, &mut storage as *mut _ as *mut libc::sockaddr, &mut len) };
    if rc != 0 {
        return None;
    }
    let len = len as usize;
    match storage.ss_family as libc::c_int {
        libc::AF_INET if len >= std::mem::size_of::<libc::sockaddr_in>() => {
            // SAFETY: family is AF_INET and the kernel wrote at least a full
            // sockaddr_in, so this reinterpret reads only initialized bytes.
            let a = unsafe { &*(&storage as *const _ as *const libc::sockaddr_in) };
            // `s_addr` is network byte order; its in-memory bytes are the
            // dotted-quad octets, which `to_ne_bytes` returns verbatim.
            Some(IpAddr::V4(Ipv4Addr::from(a.sin_addr.s_addr.to_ne_bytes())))
        }
        libc::AF_INET6 if len >= std::mem::size_of::<libc::sockaddr_in6>() => {
            // SAFETY: family is AF_INET6 with a full sockaddr_in6 written.
            let a = unsafe { &*(&storage as *const _ as *const libc::sockaddr_in6) };
            Some(IpAddr::V6(Ipv6Addr::from(a.sin6_addr.s6_addr)))
        }
        _ => None,
    }
}

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, StudioConfig};
use rusqlite::Connection;

// ---------------------------------------------------------------------------
// Encryption helpers — used by Runtime constructors + Runtime methods that
// touch encrypted fields (insert/update/get_by_id/list/list_after/lookup).
// ---------------------------------------------------------------------------

/// Build `entity → [encrypted field names]` from the manifest. Called
/// once per `Runtime::open` and cached. Entities with no encrypted
/// fields don't appear; lookup is `Option<&Vec<String>>`.
fn encryption_field_map(
    entities: &HashMap<String, ManifestEntity>,
) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    for (name, entity) in entities {
        let enc: Vec<String> = entity
            .fields
            .iter()
            .filter(|f| f.encrypted)
            .map(|f| f.name.clone())
            .collect();
        if !enc.is_empty() {
            out.insert(name.clone(), enc);
        }
    }
    out
}

/// Load the `PYLON_ENCRYPTION_KEY` env. Returns:
/// - `Ok(None)` when the env is unset (apps without encrypted fields)
/// - `Ok(Some(key))` when the env decodes to a valid 32-byte key
/// - `Err(RuntimeError)` when the env is set but malformed —
///   FAILS BOOT. Codex called out that the previous "log + continue"
///   behavior shipped a process that looks healthy but rejects
///   every encrypted-field write at runtime.
fn load_encryption_key() -> Result<Option<encryption::EncryptionKey>, RuntimeError> {
    encryption::EncryptionKey::from_env().map_err(|e| RuntimeError {
        code: "ENCRYPTION_KEY_INVALID".into(),
        message: format!(
            "PYLON_ENCRYPTION_KEY is set but invalid: {e}. \
             Fix the env (32-byte hex or base64) or unset it."
        ),
    })
}

/// Inject the framework-managed `_Connection` entity into the
/// manifest when the app declares any `connections:`. Idempotent —
/// if an `_Connection` entity already exists (apps that want to
/// extend it shouldn't have to, but we don't fight them), we leave
/// it alone.
/// Force `server_only` on every `vector(dims)` field. The SDK's
/// `entitiesToManifest` already emits it, but a hand-written manifest
/// JSON could omit the flag — and then multi-KB embeddings would ride
/// HTTP entity reads, sync snapshots, and WS change events. The flag is
/// non-negotiable for vector fields, so normalize at load rather than
/// trusting the producer.
fn force_vector_fields_server_only(manifest: &mut AppManifest) {
    for entity in &mut manifest.entities {
        for field in &mut entity.fields {
            if pylon_storage::vector::vector_dims(&field.field_type).is_some() && !field.server_only
            {
                tracing::warn!(
                    "[manifest] {}.{} is vector-typed but not serverOnly; forcing serverOnly (embeddings never leave the server)",
                    entity.name,
                    field.name
                );
                field.server_only = true;
            }
        }
    }
}

fn ensure_connection_entity(manifest: &mut AppManifest) {
    if manifest.connections.is_empty() {
        return;
    }
    if manifest.entities.iter().any(|e| e.name == "_Connection") {
        return;
    }
    manifest.entities.push(connections::connection_entity());
}

/// Inject the framework-managed `_CronLease` entity when the app
/// declares any `crons:`. This entity backs the per-(cron, minute)
/// single-fire lease so an app scaled to N replicas runs each cron
/// exactly once per tick instead of N times. Default-deny (no
/// policy) keeps it server-only — it never syncs to clients.
/// Idempotent: leave any existing `_CronLease` alone.
fn ensure_cron_lease_entity(manifest: &mut AppManifest) {
    if manifest.crons.is_empty() {
        return;
    }
    if manifest.entities.iter().any(|e| e.name == "_CronLease") {
        return;
    }
    manifest
        .entities
        .push(crate::datastore::cron_lease_entity());
}

/// Bootstrap the framework-internal `_CronLease` table on Postgres.
///
/// `open_postgres` deliberately does NOT auto-create app tables — schema is
/// applied via `pylon migrate`, which reads the app's `pylon.manifest.json`.
/// But `_CronLease` is injected at runtime boot and never appears in that
/// file, so `pylon migrate` would never create it. Mirror the CRDT-sidecar
/// bootstrap a few lines up: run idempotent `CREATE TABLE / INDEX IF NOT
/// EXISTS` on every open. Without this, the cross-replica lease insert in
/// `claim_cron_lease` hits a missing relation, fails open, and every replica
/// fires the cron — exactly the behavior the lease exists to prevent.
fn ensure_cron_lease_table_pg(
    store: &pylon_storage::pg_datastore::PostgresDataStore,
) -> Result<(), RuntimeError> {
    let entity = crate::datastore::cron_lease_entity();
    let fields: Vec<pylon_storage::FieldSpec> = entity
        .fields
        .iter()
        .map(|f| pylon_storage::FieldSpec {
            name: f.name.clone(),
            field_type: f.field_type.clone(),
            optional: f.optional,
            unique: f.unique,
        })
        .collect();
    let mut stmts = vec![pylon_storage::postgres::create_table_sql(
        &entity.name,
        &fields,
    )];
    for idx in &entity.indexes {
        stmts.push(pylon_storage::postgres::create_index_sql(
            &entity.name,
            &idx.name,
            &idx.fields,
            idx.unique,
            idx.where_clause.as_deref(),
        ));
    }
    store
        .with_client(|c| {
            for stmt in &stmts {
                c.execute(stmt.as_str(), &[])
                    .map_err(|e| pylon_http::DataError {
                        code: "PG_EXEC_FAILED".into(),
                        message: format!("{e}"),
                    })?;
            }
            Ok::<(), pylon_http::DataError>(())
        })
        .map_err(data_err_to_runtime)
}

/// When an app declares connections, `PYLON_ENCRYPTION_KEY` is
/// REQUIRED — refresh tokens never live in plaintext. Codex P2
/// fix: fail boot with a clear error rather than letting the
/// runtime serve broken auth-url calls.
fn validate_connection_encryption_present(
    manifest: &AppManifest,
    key: &Option<encryption::EncryptionKey>,
) -> Result<(), RuntimeError> {
    if !manifest.connections.is_empty() && key.is_none() {
        return Err(RuntimeError {
            code: "CONNECTIONS_REQUIRE_ENCRYPTION".into(),
            message: format!(
                "Manifest declares {} connection(s) but PYLON_ENCRYPTION_KEY is unset. \
                 Refresh tokens must be encrypted at rest.",
                manifest.connections.len()
            ),
        });
    }
    Ok(())
}

/// Validate that every manifest field marked `encrypted: true`
/// meets the documented restrictions:
/// - field type is `string` or `richtext` (random-nonce encryption
///   produces opaque bytes; numeric/bool columns would corrupt)
/// - `unique: true` is NOT set (random nonce defeats uniqueness)
/// - `server_only: true` is set (encrypted plaintext must never
///   reach the wire — without `serverOnly`, decrypted values would
///   ship over every public HTTP/WS surface)
///
/// Returns a `RuntimeError` listing every violation found. Boot
/// fails on the first violating manifest so deploys reject mis-
/// configurations instead of running with a silently-broken
/// invariant.
fn validate_encrypted_fields(manifest: &AppManifest) -> Result<(), RuntimeError> {
    let mut violations: Vec<String> = Vec::new();
    for entity in &manifest.entities {
        for field in &entity.fields {
            if !field.encrypted {
                continue;
            }
            let ent = &entity.name;
            let name = &field.name;
            let ftype = &field.field_type;
            // Only string-shaped types. The encryption pre-pass
            // always produces a base64 string on the wire; a column
            // typed `int`/`bool`/`datetime`/`float` either rejects
            // (Postgres) or silently corrupts (SQLite via type
            // affinity).
            if !matches!(ftype.as_str(), "string" | "richtext") {
                violations.push(format!(
                    "{ent}.{name}: encrypted() requires field type `string` or `richtext` (got `{ftype}`)"
                ));
            }
            if field.unique {
                violations.push(format!(
                    "{ent}.{name}: encrypted() cannot combine with unique() — random-nonce encryption defeats uniqueness"
                ));
            }
            if !field.server_only {
                violations.push(format!(
                    "{ent}.{name}: encrypted() requires serverOnly() — decrypted plaintext must never reach HTTP/WS responses"
                ));
            }
        }
    }
    if !violations.is_empty() {
        return Err(RuntimeError {
            code: "ENCRYPTION_MANIFEST_INVALID".into(),
            message: format!(
                "Manifest declares encrypted fields with invalid combinations:\n  - {}",
                violations.join("\n  - ")
            ),
        });
    }
    Ok(())
}

fn validate_manifest_org_roles(manifest: &AppManifest) -> Result<(), RuntimeError> {
    pylon_kernel::validate_org_roles(&manifest.auth.org_roles).map_err(|message| RuntimeError {
        code: "BAD_ORG_ROLE".into(),
        message,
    })
}

// ---------------------------------------------------------------------------
// Runtime errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for RuntimeError {}

/// Lift a `DataError` (the cross-crate error type for PG `DataStore`
/// operations) into a `RuntimeError`. Used by `PostgresDataStore`
/// closure bounds (`with_client`, `with_transaction`) so callers in
/// the runtime can propagate PG errors with their native error type.
impl From<pylon_http::DataError> for RuntimeError {
    fn from(e: pylon_http::DataError) -> Self {
        RuntimeError {
            code: e.code,
            message: e.message,
        }
    }
}

// ---------------------------------------------------------------------------
// SQL safety helpers
// ---------------------------------------------------------------------------

/// Quote a SQL identifier with double quotes to prevent injection.
/// Any embedded double quotes are escaped by doubling them (SQL standard).
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Validate that `name` is a known column on the given entity.
/// Always allows "id" (the primary key). Returns an error listing valid
/// columns when validation fails.
fn validate_column_name(name: &str, entity: &ManifestEntity) -> Result<(), RuntimeError> {
    if name == "id" {
        return Ok(());
    }
    if entity.fields.iter().any(|f| f.name == name) {
        return Ok(());
    }
    Err(RuntimeError {
        code: "INVALID_COLUMN".into(),
        message: format!(
            "Unknown column \"{}\" -- valid columns: id, {}",
            name,
            entity
                .fields
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

// ---------------------------------------------------------------------------
// Connection tuning
// ---------------------------------------------------------------------------

/// Apply the production pragma set on a SQLite connection. Identical
/// values to `pylon_storage::sqlite::tune_connection` — kept here too
/// because the Runtime opens its own connections directly (write +
/// read pool) without going through the storage adapter.
///
/// See `crates/storage/src/sqlite.rs` for the rationale on each
/// pragma. Skipping it on writes drops throughput by 5–10×.
///
/// Returns the first pragma that failed so the caller can decide
/// fatal-vs-lenient. Boot paths (every backend's `new()`, the main
/// runtime's open) MUST surface this error so a silent pragma miss
/// can't ever produce a half-tuned DB that hangs on the first
/// busy_timeout-less lock wait. Run-time / hot-path callers (read-
/// pool clones, etc.) can swallow with `let _ = ...` when the
/// failure mode is "log and proceed."
pub(crate) fn tune_runtime_connection(
    conn: &Connection,
    in_memory: bool,
) -> Result<(), rusqlite::Error> {
    let pragmas: &[(&str, &str)] = if in_memory {
        &[
            ("temp_store", "MEMORY"),
            ("cache_size", "-65536"),
            ("foreign_keys", "ON"),
        ]
    } else {
        &[
            ("journal_mode", "WAL"),
            ("synchronous", "NORMAL"),
            ("cache_size", "-65536"),
            ("mmap_size", "268435456"),
            ("temp_store", "MEMORY"),
            ("busy_timeout", "5000"),
            ("foreign_keys", "ON"),
            ("wal_autocheckpoint", "1000"),
        ]
    };
    for (key, value) in pragmas {
        conn.pragma_update(None, key, value)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Read connection guard
// ---------------------------------------------------------------------------

/// A guard that dereferences to a `Connection`, abstracting over whether
/// it came from the read pool or fell back to the write connection.
enum ReadConnGuard<'a> {
    Pooled(std::sync::MutexGuard<'a, Connection>),
    Write(std::sync::MutexGuard<'a, Connection>),
}

impl<'a> std::ops::Deref for ReadConnGuard<'a> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        match self {
            ReadConnGuard::Pooled(g) => g,
            ReadConnGuard::Write(g) => g,
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime — the core execution engine
// ---------------------------------------------------------------------------

/// A manifest-driven runtime that executes CRUD operations against an
/// underlying data store. Two backends are supported:
///
/// - **SQLite** (default): single-process, file-or-memory, with a write
///   mutex + read pool, FTS5 search, and per-row LoroDoc CRDT snapshots.
/// - **Postgres**: live cluster, suitable for multi-replica deployments.
///   Routes entity CRUD through [`pylon_storage::pg_datastore::PostgresDataStore`].
///   CRDT mode and FTS5-shaped search are SQLite-only at this layer; the
///   Postgres backend returns `NOT_SUPPORTED` for those paths and the router
///   degrades to JSON change events (no binary CRDT broadcasts).
///
/// Pick a backend by passing a `postgres://` URL to [`Runtime::open`]; any
/// other string is treated as a SQLite filesystem path.
pub struct Runtime {
    backend: RuntimeBackend,
    /// `Arc` so the per-call clone on the transactional-function path is a
    /// refcount bump, not a deep copy of every entity/field/policy (see
    /// `manifest_arc`). Field access still derefs transparently to
    /// `&AppManifest`.
    manifest: Arc<AppManifest>,
    entities: HashMap<String, ManifestEntity>,
    /// True only for the SQLite in-memory variant. Postgres mode reports false.
    /// Gates the test-reset endpoint — a false positive here would let
    /// `/api/__test__/reset` truncate real tables.
    is_in_memory: bool,
    /// Path to the user's `.pylon/studio.config.json`, populated by the
    /// CLI when `studio.config.ts` is present in the project. The
    /// `/studio` handler re-reads this file on every render so dev
    /// edits don't require a server restart. `None` means "no config
    /// authored — use defaults."
    ///
    /// Wrapped in a `RwLock` so the dev watch loop can swap the path
    /// without taking down the server (e.g. when the operator first
    /// adds a `studio.config.ts` to a running project). Reads are
    /// cheap; writes happen at most once per dev cycle.
    studio_config_path: RwLock<Option<PathBuf>>,
    /// Path to the bundled `.pylon/studio.entry.js` if the project
    /// ships custom Studio extensions. Same hot-swap semantics as
    /// `studio_config_path`.
    studio_entry_path: RwLock<Option<PathBuf>>,
    /// AEAD key for `field.encrypted()` fields. Loaded once at boot
    /// from `PYLON_ENCRYPTION_KEY` env. `None` means encryption is
    /// not configured — reads/writes to encrypted fields skip the
    /// crypto step but the runtime logs a warning per write when the
    /// manifest declares encrypted fields without a key (operator
    /// surface: see `docs/security/encryption.md`).
    encryption_key: Option<encryption::EncryptionKey>,
    /// Cached `entity → encrypted field names` map, computed once
    /// from the manifest. Per-row encrypt/decrypt looks the entity
    /// up here to avoid scanning the manifest on the hot path.
    encrypted_fields: HashMap<String, Vec<String>>,
    /// Shared ConnectionManager — `None` when the manifest declares
    /// no connections. Codex P1: this MUST be a single Arc shared
    /// across boot, HTTP routes, and the function hook, so the
    /// CSRF state token minted in /auth-url is observable from
    /// /callback. Per-call construction breaks the flow.
    connection_manager: std::sync::OnceLock<Option<std::sync::Arc<connections::ConnectionManager>>>,
}

/// Backend storage for entity CRUD. SQLite variant owns the connection
/// pool and CRDT cache; Postgres variant wraps a `PostgresDataStore`.
enum RuntimeBackend {
    Sqlite(SqliteBackend),
    Postgres(PgBackend),
}

/// SQLite-backed entity store. WAL mode allows one writer and multiple
/// concurrent readers — the struct exploits this with a single write
/// connection behind a mutex plus a pool of read-only connections.
struct SqliteBackend {
    /// Write connection — single mutex, serializes writes.
    write_conn: Mutex<Connection>,
    /// Read connections — pool of connections for concurrent reads.
    /// Empty for in-memory databases where extra connections are not possible.
    read_pool: Vec<Mutex<Connection>>,
    /// Counter for round-robin read pool selection.
    read_counter: AtomicUsize,
    /// Per-row LoroDoc cache + sidecar persistence. Used for entities with
    /// `crdt: true` (the default). Reads still hit SQLite directly via the
    /// read pool — the LoroDoc just produces the projected JSON that gets
    /// materialized into SQLite columns on every write.
    crdt: crate::loro_store::LoroStore,
}

/// Postgres-backed entity store. Wraps `PostgresDataStore` from
/// pylon-storage and delegates the `DataStore` surface directly.
pub(crate) struct PgBackend {
    pub(crate) store: pylon_storage::pg_datastore::PostgresDataStore,
    /// Per-row LoroDoc snapshot store for entities with `crdt: true`.
    /// Arc'd so the runtime layer can hand a clone to PgCrdtHookImpl
    /// (the bridge that lets PgTxStore call back into CRDT machinery
    /// from inside a held tx).
    pub(crate) crdt: std::sync::Arc<crate::pg_loro_store::PgLoroStore>,
}

/// Number of read-only connections to open in the pool.
const READ_POOL_SIZE: usize = 4;

/// True iff `url` is a Postgres connection string. Treats `postgres://`,
/// `postgresql://`, and the ambient-credentials forms as PG; everything
/// else is interpreted as a SQLite filesystem path.
fn is_postgres_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("postgres://") || lower.starts_with("postgresql://")
}

/// Convert a `pylon_http::DataError` (returned by `PostgresDataStore`)
/// into the runtime's error type. The codes round-trip; only the type
/// changes.
fn data_err_to_runtime(e: pylon_http::DataError) -> RuntimeError {
    RuntimeError {
        code: e.code,
        message: e.message,
    }
}

/// Map a `pylon_sync::ChangeKind` to the string form persisted in
/// `_pylon_change_log.kind`. The strings are stable wire format —
/// don't rename them without a migration.
fn change_kind_to_str(kind: pylon_sync::ChangeKind) -> &'static str {
    match kind {
        pylon_sync::ChangeKind::Insert => "insert",
        pylon_sync::ChangeKind::Update => "update",
        pylon_sync::ChangeKind::Delete => "delete",
    }
}

fn change_kind_from_str(s: &str) -> Option<pylon_sync::ChangeKind> {
    match s {
        "insert" => Some(pylon_sync::ChangeKind::Insert),
        "update" => Some(pylon_sync::ChangeKind::Update),
        "delete" => Some(pylon_sync::ChangeKind::Delete),
        _ => None,
    }
}

/// Postgres row → `Option<ChangeEvent>`. Counterpart to the
/// rusqlite mapper below.
fn pg_row_to_change_event(row: &postgres::Row) -> Option<pylon_sync::ChangeEvent> {
    let seq: i64 = row.try_get(0).ok()?;
    let entity: String = row.try_get(1).ok()?;
    let row_id: String = row.try_get(2).ok()?;
    let kind_str: String = row.try_get(3).ok()?;
    let kind = change_kind_from_str(&kind_str)?;
    let data: Option<serde_json::Value> = row.try_get(4).ok();
    let prev_data: Option<serde_json::Value> = row.try_get(5).ok();
    let timestamp: String = row.try_get(6).ok()?;
    Some(pylon_sync::ChangeEvent {
        seq: seq as u64,
        entity,
        row_id,
        kind,
        data,
        prev_data,
        timestamp,
    })
}

/// rusqlite row → `Option<ChangeEvent>` mapper shared by the two
/// SELECT helpers. Returns `None` if the persisted `kind` doesn't
/// match a known variant (would indicate schema drift).
fn change_log_row_map(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Option<pylon_sync::ChangeEvent>> {
    let seq: i64 = row.get(0)?;
    let entity: String = row.get(1)?;
    let row_id: String = row.get(2)?;
    let kind_str: String = row.get(3)?;
    let data_blob: Option<Vec<u8>> = row.get(4)?;
    let prev_blob: Option<Vec<u8>> = row.get(5)?;
    let timestamp: String = row.get(6)?;
    let Some(kind) = change_kind_from_str(&kind_str) else {
        return Ok(None);
    };
    let data = data_blob.and_then(|b| serde_json::from_slice(&b).ok());
    let prev_data = prev_blob.and_then(|b| serde_json::from_slice(&b).ok());
    Ok(Some(pylon_sync::ChangeEvent {
        seq: seq as u64,
        entity,
        row_id,
        kind,
        data,
        prev_data,
        timestamp,
    }))
}

impl Runtime {
    /// Open a runtime against either a SQLite file path or a Postgres URL.
    ///
    /// Backend selection is by URL prefix:
    /// - `postgres://...` or `postgresql://...` → Postgres (requires the
    ///   `postgres-live` feature on `pylon-storage`, enabled by default).
    /// - Anything else → SQLite, treating the string as a filesystem path
    ///   (`":memory:"` works via `Runtime::in_memory` instead).
    pub fn open(url: &str, mut manifest: AppManifest) -> Result<Self, RuntimeError> {
        if is_postgres_url(url) {
            Self::open_postgres(url, manifest)
        } else {
            let conn = Connection::open(url).map_err(|e| RuntimeError {
                code: "RUNTIME_OPEN_FAILED".into(),
                message: format!("Failed to open database: {e}"),
            })?;
            Self::from_connection(conn, manifest, false)
        }
    }

    /// Open a runtime backed by a live Postgres cluster.
    ///
    /// Schema must be applied separately via `pylon migrate` / the
    /// storage adapter's plan apply path — Runtime does not auto-create
    /// tables on Postgres (in contrast to SQLite, where `from_connection`
    /// runs CREATE TABLE IF NOT EXISTS on every open). This matches how
    /// production Postgres deployments are typically managed: schema is
    /// migrated via a controlled, observable step, not as a side effect
    /// of the server starting up.
    pub fn open_postgres(url: &str, mut manifest: AppManifest) -> Result<Self, RuntimeError> {
        // Inject framework-managed entities BEFORE building the store. The
        // PostgresDataStore snapshots the manifest at construction and validates
        // every insert/query entity name against that snapshot — so anything
        // injected AFTER `connect` is invisible to the store, and every
        // `_Connection`/`_CronLease` op fails `ENTITY_NOT_FOUND`. For the cron
        // lease that's silent double-fire: the insert + the owner read-back both
        // error, and a non-conflict error fails open to `Run` on every replica.
        // (SQLite has no separate store snapshot — `from_connection` validates
        // against the runtime's own manifest — so this only bites Postgres.)
        ensure_connection_entity(&mut manifest);
        ensure_cron_lease_entity(&mut manifest);
        force_vector_fields_server_only(&mut manifest);
        validate_manifest_org_roles(&manifest)?;
        // Serialize this machine's boot DDL against peers sharing the
        // database (see pg_boot_guard) — released by start_server once
        // every backend has bootstrapped its tables.
        crate::pg_boot_guard::acquire(url).map_err(|e| RuntimeError {
            code: "BOOT_DDL_GUARD_FAILED".into(),
            message: e,
        })?;
        let store = pylon_storage::pg_datastore::PostgresDataStore::connect(url, manifest.clone())
            .map_err(data_err_to_runtime)?;
        // Bootstrap the CRDT sidecar table on every open. Idempotent,
        // and race-free under the boot-DDL guard acquired above.
        store
            .with_client(|c| crate::pg_loro_store::ensure_sidecar(c))
            .map_err(|e| RuntimeError {
                code: "CRDT_SIDECAR_BOOTSTRAP_FAILED".into(),
                message: format!("ensure pg crdt sidecar: {e}"),
            })?;
        // `pylon migrate` never sees the injected `_CronLease` (it's not in the
        // app's manifest file), so create its table here — idempotently — the
        // same way the CRDT sidecar is bootstrapped above. SQLite gets it for
        // free via `from_connection`'s CREATE TABLE IF NOT EXISTS pass.
        if !manifest.crons.is_empty() {
            ensure_cron_lease_table_pg(&store)?;
        }
        validate_encrypted_fields(&manifest)?;
        // Encryption-key check happens after key load — see below.
        let entities: HashMap<String, ManifestEntity> = manifest
            .entities
            .iter()
            .map(|e| (e.name.clone(), e.clone()))
            .collect();
        let encrypted_fields = encryption_field_map(&entities);
        let encryption_key = load_encryption_key()?;
        validate_connection_encryption_present(&manifest, &encryption_key)?;
        Ok(Self {
            backend: RuntimeBackend::Postgres(PgBackend {
                store,
                crdt: std::sync::Arc::new(crate::pg_loro_store::PgLoroStore::new()),
            }),
            manifest: Arc::new(manifest),
            entities,
            is_in_memory: false,
            studio_config_path: RwLock::new(None),
            studio_entry_path: RwLock::new(None),
            encryption_key,
            encrypted_fields,
            connection_manager: std::sync::OnceLock::new(),
        })
    }

    /// Returns true if this runtime is backed by an in-memory SQLite DB.
    ///
    /// Stored at open time rather than queried via `conn.path()` because
    /// the path-based check conflates "no filename" with "in-memory":
    /// `Connection::open("")` yields a file-backed DB with empty path,
    /// and would falsely pass as in-memory. Since we always know at
    /// construction time which constructor was used, track the bit.
    ///
    /// Gates the test-reset endpoint — a false positive here would let
    /// `/api/__test__/reset` truncate real tables.
    pub fn is_in_memory(&self) -> bool {
        self.is_in_memory
    }

    /// Bootstrap a globally-monotonic change-log SEQUENCE on Postgres.
    /// Idempotent — `CREATE SEQUENCE IF NOT EXISTS` runs once per
    /// process boot. Returns `Ok(())` on SQLite (no-op) so callers
    /// don't have to branch on backend.
    ///
    /// Cluster mode: every instance shares this single sequence so
    /// `append`'s `nextval` returns globally-unique, monotonically-
    /// increasing seqs. Without it, instance A and instance B can
    /// independently emit seq=5, and clients drop one as a duplicate
    /// of the other (codex P1).
    pub fn bootstrap_global_change_seq(&self) -> Result<(), RuntimeError> {
        let RuntimeBackend::Postgres(pg) = &self.backend else {
            return Ok(());
        };
        pg.store
            .with_client(|c| {
                c.execute(
                    "CREATE SEQUENCE IF NOT EXISTS pylon_change_seq START 1",
                    &[],
                )
                .map_err(|e| pylon_http::DataError {
                    code: "PG_SEQUENCE_BOOTSTRAP_FAILED".into(),
                    message: format!("CREATE SEQUENCE pylon_change_seq: {e}"),
                })?;
                Ok::<(), pylon_http::DataError>(())
            })
            .map_err(data_err_to_runtime)
    }

    /// Mint the next global change-log seq. Postgres-only — SQLite
    /// returns `None` and callers should fall back to the local
    /// atomic counter.
    pub fn next_global_change_seq(&self) -> Option<u64> {
        let RuntimeBackend::Postgres(pg) = &self.backend else {
            return None;
        };
        pg.store
            .with_client(|c| {
                let row = c
                    .query_one("SELECT nextval('pylon_change_seq')", &[])
                    .map_err(|e| pylon_http::DataError {
                        code: "PG_SEQUENCE_NEXTVAL_FAILED".into(),
                        message: e.to_string(),
                    })?;
                let v: i64 = row.get(0);
                Ok::<u64, pylon_http::DataError>(v as u64)
            })
            .ok()
    }

    /// Snapshot the current SEQUENCE value (without minting a new one)
    /// so the change-log's local seq counter starts at the correct
    /// position after process restart. Postgres-only.
    pub fn current_global_change_seq(&self) -> Option<u64> {
        let RuntimeBackend::Postgres(pg) = &self.backend else {
            return None;
        };
        pg.store
            .with_client(|c| {
                let row = c
                    .query_one("SELECT last_value, is_called FROM pylon_change_seq", &[])
                    .map_err(|e| pylon_http::DataError {
                        code: "PG_SEQUENCE_LASTVAL_FAILED".into(),
                        message: e.to_string(),
                    })?;
                let last_value: i64 = row.get(0);
                let is_called: bool = row.get(1);
                // A fresh sequence reports `last_value=1, is_called=false`
                // meaning "next nextval() returns 1". Normalize to 0 so
                // the change-log doesn't claim seq=1 prematurely.
                let v = if is_called { last_value } else { 0 };
                Ok::<u64, pylon_http::DataError>(v as u64)
            })
            .ok()
    }

    /// Create the `pylon_change_log` table in Postgres if it doesn't
    /// exist. Postgres analog of `bootstrap_sqlite_change_log`.
    /// Idempotent. Called once at boot for Pg-backed runtimes.
    pub fn bootstrap_pg_change_log(&self) -> Result<(), RuntimeError> {
        let RuntimeBackend::Postgres(pg) = &self.backend else {
            return Ok(());
        };
        pg.store
            .with_client(|c| {
                c.execute(
                    "CREATE TABLE IF NOT EXISTS pylon_change_log (\
                     seq BIGINT PRIMARY KEY,\
                     entity TEXT NOT NULL,\
                     row_id TEXT NOT NULL,\
                     kind TEXT NOT NULL,\
                     data JSONB,\
                     prev_data JSONB,\
                     ts TEXT NOT NULL\
                     )",
                    &[],
                )
                .map_err(|e| pylon_http::DataError {
                    code: "PG_CHANGE_LOG_BOOTSTRAP_FAILED".into(),
                    message: format!("CREATE TABLE pylon_change_log: {e}"),
                })?;
                c.execute(
                    "CREATE INDEX IF NOT EXISTS pylon_change_log_entity_seq \
                     ON pylon_change_log(entity, seq)",
                    &[],
                )
                .map_err(|e| pylon_http::DataError {
                    code: "PG_CHANGE_LOG_BOOTSTRAP_FAILED".into(),
                    message: format!("CREATE INDEX: {e}"),
                })?;
                Ok::<(), pylon_http::DataError>(())
            })
            .map_err(data_err_to_runtime)
    }

    /// Persist a batch of events to `pylon_change_log`. Postgres
    /// analog of `sqlite_change_log_persist_batch`. Uses a single
    /// transaction + a prepared statement for batched INSERT.
    pub fn pg_change_log_persist_batch(
        &self,
        events: &[pylon_sync::ChangeEvent],
    ) -> Result<(), RuntimeError> {
        if events.is_empty() {
            return Ok(());
        }
        let RuntimeBackend::Postgres(pg) = &self.backend else {
            return Ok(());
        };
        pg.store
            .with_client(|c| {
                let mut tx = c.transaction().map_err(|e| pylon_http::DataError {
                    code: "PG_CHANGE_LOG_TX_BEGIN_FAILED".into(),
                    message: e.to_string(),
                })?;
                let stmt = tx
                    .prepare(
                        "INSERT INTO pylon_change_log \
                         (seq, entity, row_id, kind, data, prev_data, ts) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7) \
                         ON CONFLICT (seq) DO NOTHING",
                    )
                    .map_err(|e| pylon_http::DataError {
                        code: "PG_CHANGE_LOG_PREPARE_FAILED".into(),
                        message: e.to_string(),
                    })?;
                for event in events {
                    let data = event.data.clone();
                    let prev_data = event.prev_data.clone();
                    tx.execute(
                        &stmt,
                        &[
                            &(event.seq as i64),
                            &event.entity,
                            &event.row_id,
                            &change_kind_to_str(event.kind.clone()),
                            &data,
                            &prev_data,
                            &event.timestamp,
                        ],
                    )
                    .map_err(|e| pylon_http::DataError {
                        code: "PG_CHANGE_LOG_INSERT_FAILED".into(),
                        message: format!("INSERT seq={}: {e}", event.seq),
                    })?;
                }
                tx.commit().map_err(|e| pylon_http::DataError {
                    code: "PG_CHANGE_LOG_COMMIT_FAILED".into(),
                    message: e.to_string(),
                })?;
                Ok::<(), pylon_http::DataError>(())
            })
            .map_err(data_err_to_runtime)
    }

    /// Read recent events from `pylon_change_log` ordered ASC. Used at
    /// boot to hydrate the in-memory ring.
    pub fn pg_change_log_load_recent(&self, limit: usize) -> Vec<pylon_sync::ChangeEvent> {
        let RuntimeBackend::Postgres(pg) = &self.backend else {
            return Vec::new();
        };
        let result: Result<Vec<pylon_sync::ChangeEvent>, pylon_http::DataError> =
            pg.store.with_client(|c| {
                let rows = c
                    .query(
                        "SELECT seq, entity, row_id, kind, data, prev_data, ts \
                         FROM (SELECT seq, entity, row_id, kind, data, prev_data, ts \
                               FROM pylon_change_log ORDER BY seq DESC LIMIT $1) sub \
                         ORDER BY seq ASC",
                        &[&(limit as i64)],
                    )
                    .map_err(|e| pylon_http::DataError {
                        code: "PG_CHANGE_LOG_LOAD_FAILED".into(),
                        message: e.to_string(),
                    })?;
                let mut out = Vec::with_capacity(rows.len());
                for r in rows {
                    if let Some(ev) = pg_row_to_change_event(&r) {
                        out.push(ev);
                    }
                }
                Ok(out)
            });
        result.unwrap_or_default()
    }

    /// Read events `seq > since` from `pylon_change_log`. Backs the
    /// pull-fallback path when the in-memory ring no longer covers
    /// the cursor.
    /// Read persisted change-log events with `seq > since`, oldest first.
    /// Returns `None` on a backend mismatch or query failure so the
    /// caller won't mistake a transient PG error for a confirmed-empty
    /// (pruned) gap and force a resync storm. See
    /// [`pylon_sync::ChangeLogStore::pull_range`].
    pub fn pg_change_log_pull_range(
        &self,
        since: u64,
        limit: usize,
    ) -> Option<Vec<pylon_sync::ChangeEvent>> {
        let RuntimeBackend::Postgres(pg) = &self.backend else {
            return None;
        };
        let result: Result<Vec<pylon_sync::ChangeEvent>, pylon_http::DataError> =
            pg.store.with_client(|c| {
                let rows = c
                    .query(
                        "SELECT seq, entity, row_id, kind, data, prev_data, ts \
                         FROM pylon_change_log WHERE seq > $1 ORDER BY seq ASC LIMIT $2",
                        &[&(since as i64), &(limit as i64)],
                    )
                    .map_err(|e| pylon_http::DataError {
                        code: "PG_CHANGE_LOG_PULL_FAILED".into(),
                        message: e.to_string(),
                    })?;
                let mut out = Vec::with_capacity(rows.len());
                for r in rows {
                    if let Some(ev) = pg_row_to_change_event(&r) {
                        out.push(ev);
                    }
                }
                Ok(out)
            });
        result.ok()
    }

    /// Prune `pylon_change_log` to at most `retain` events.
    pub fn pg_change_log_prune(&self, retain: u64) -> Result<u64, RuntimeError> {
        let RuntimeBackend::Postgres(pg) = &self.backend else {
            return Ok(0);
        };
        pg.store
            .with_client(|c| {
                let row = c
                    .query_one("SELECT MAX(seq) FROM pylon_change_log", &[])
                    .map_err(|e| pylon_http::DataError {
                        code: "PG_CHANGE_LOG_PRUNE_QUERY_FAILED".into(),
                        message: e.to_string(),
                    })?;
                let max_seq: Option<i64> = row.get(0);
                let max_seq = match max_seq {
                    Some(m) => m,
                    None => return Ok(0u64),
                };
                let cutoff = max_seq.saturating_sub(retain as i64);
                if cutoff <= 0 {
                    return Ok(0u64);
                }
                let n = c
                    .execute("DELETE FROM pylon_change_log WHERE seq <= $1", &[&cutoff])
                    .map_err(|e| pylon_http::DataError {
                        code: "PG_CHANGE_LOG_PRUNE_FAILED".into(),
                        message: e.to_string(),
                    })?;
                Ok(n)
            })
            .map_err(data_err_to_runtime)
    }

    /// Has any event for the named entity been persisted?
    pub fn pg_change_log_has_entity(&self, entity: &str) -> bool {
        let RuntimeBackend::Postgres(pg) = &self.backend else {
            return false;
        };
        pg.store
            .with_client(|c| {
                let row = c
                    .query_opt(
                        "SELECT 1 FROM pylon_change_log WHERE entity = $1 LIMIT 1",
                        &[&entity],
                    )
                    .map_err(|e| pylon_http::DataError {
                        code: "PG_CHANGE_LOG_HAS_ENTITY_FAILED".into(),
                        message: e.to_string(),
                    })?;
                Ok::<bool, pylon_http::DataError>(row.is_some())
            })
            .unwrap_or(false)
    }

    /// SQLite analog of `bootstrap_global_change_seq`. Creates a
    /// `_pylon_change_seq` row whose single `value` column carries the
    /// highest seq the server has ever minted. Without this, every
    /// process restart resets the in-memory seq counter to 0 + seeds it
    /// to ~N (the current entity count). Any client whose cursor is
    /// ahead of the new seed range gets a permanent 410 RESYNC_REQUIRED
    /// from `crates/sync/src/lib.rs:568` because `cursor.last_seq >
    /// current_seq`. The 410 forces the client through reset → re-pull,
    /// which is visible UI churn on every deploy. Persisting the seq
    /// across restarts eliminates the case.
    pub fn bootstrap_sqlite_change_seq(&self) -> Result<(), RuntimeError> {
        let sb = self.sqlite_backend()?;
        let conn = sb.write_conn.lock().map_err(|e| RuntimeError {
            code: "SQLITE_LOCK_FAILED".into(),
            message: format!("write_conn lock poisoned: {e}"),
        })?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS _pylon_change_seq (id INTEGER PRIMARY KEY CHECK (id = 1), value INTEGER NOT NULL)",
            [],
        )
        .map_err(|e| RuntimeError {
            code: "SQLITE_SEQUENCE_BOOTSTRAP_FAILED".into(),
            message: format!("CREATE TABLE _pylon_change_seq: {e}"),
        })?;
        conn.execute(
            "INSERT OR IGNORE INTO _pylon_change_seq (id, value) VALUES (1, 0)",
            [],
        )
        .map_err(|e| RuntimeError {
            code: "SQLITE_SEQUENCE_BOOTSTRAP_FAILED".into(),
            message: format!("seed _pylon_change_seq: {e}"),
        })?;
        Ok(())
    }

    /// Snapshot the persisted seq value without minting a new one. Used
    /// at boot to align the in-memory ChangeLog counter so seed events
    /// resume from the last persisted seq instead of restarting at 0.
    pub fn current_sqlite_change_seq(&self) -> Option<u64> {
        let sb = self.sqlite_backend().ok()?;
        let conn = sb.write_conn.lock().ok()?;
        let v: i64 = conn
            .query_row(
                "SELECT value FROM _pylon_change_seq WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .ok()?;
        Some(v as u64)
    }

    /// Create the `_pylon_change_log` table if it doesn't exist.
    /// Called once at boot for SQLite deployments to enable
    /// persistent change-log replay across restarts. Idempotent.
    pub fn bootstrap_sqlite_change_log(&self) -> Result<(), RuntimeError> {
        let sb = self.sqlite_backend()?;
        let conn = sb.write_conn.lock().map_err(|e| RuntimeError {
            code: "SQLITE_LOCK_FAILED".into(),
            message: format!("write_conn lock poisoned: {e}"),
        })?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS _pylon_change_log (\
             seq INTEGER PRIMARY KEY,\
             entity TEXT NOT NULL,\
             row_id TEXT NOT NULL,\
             kind TEXT NOT NULL,\
             data BLOB,\
             prev_data BLOB,\
             timestamp TEXT NOT NULL\
             )",
            [],
        )
        .map_err(|e| RuntimeError {
            code: "SQLITE_CHANGE_LOG_BOOTSTRAP_FAILED".into(),
            message: format!("CREATE TABLE _pylon_change_log: {e}"),
        })?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS _pylon_change_log_entity_seq \
             ON _pylon_change_log(entity, seq)",
            [],
        )
        .map_err(|e| RuntimeError {
            code: "SQLITE_CHANGE_LOG_BOOTSTRAP_FAILED".into(),
            message: format!("CREATE INDEX _pylon_change_log_entity_seq: {e}"),
        })?;
        Ok(())
    }

    /// Persist a batch of change events to `_pylon_change_log` in a
    /// single SQLite transaction. Called from the background
    /// `ChangeLogPersister` worker, NOT from the mutation hot path —
    /// the hot path already holds `write_conn` for its own BEGIN/
    /// COMMIT, and trying to re-acquire here would deadlock
    /// (`std::sync::Mutex` is not reentrant — same shape as the
    /// v0.3.218 seq-persistence regression). The persister thread
    /// queues behind any active mutation tx, gets the lock when
    /// it's free, runs the batched INSERT, releases.
    pub fn sqlite_change_log_persist_batch(
        &self,
        events: &[pylon_sync::ChangeEvent],
    ) -> Result<(), RuntimeError> {
        if events.is_empty() {
            return Ok(());
        }
        let sb = self.sqlite_backend()?;
        let mut conn = sb.write_conn.lock().map_err(|e| RuntimeError {
            code: "SQLITE_LOCK_FAILED".into(),
            message: format!("write_conn lock poisoned: {e}"),
        })?;
        let tx = conn.transaction().map_err(|e| RuntimeError {
            code: "SQLITE_CHANGE_LOG_TX_BEGIN_FAILED".into(),
            message: format!("BEGIN: {e}"),
        })?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO _pylon_change_log \
                     (seq, entity, row_id, kind, data, prev_data, timestamp) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .map_err(|e| RuntimeError {
                    code: "SQLITE_CHANGE_LOG_PREPARE_FAILED".into(),
                    message: format!("prepare INSERT: {e}"),
                })?;
            for event in events {
                let data_blob = event.data.as_ref().and_then(|v| serde_json::to_vec(v).ok());
                let prev_blob = event
                    .prev_data
                    .as_ref()
                    .and_then(|v| serde_json::to_vec(v).ok());
                stmt.execute(rusqlite::params![
                    event.seq as i64,
                    event.entity,
                    event.row_id,
                    change_kind_to_str(event.kind.clone()),
                    data_blob,
                    prev_blob,
                    event.timestamp,
                ])
                .map_err(|e| RuntimeError {
                    code: "SQLITE_CHANGE_LOG_INSERT_FAILED".into(),
                    message: format!("INSERT seq={}: {e}", event.seq),
                })?;
            }
        }
        tx.commit().map_err(|e| RuntimeError {
            code: "SQLITE_CHANGE_LOG_COMMIT_FAILED".into(),
            message: format!("COMMIT: {e}"),
        })?;
        Ok(())
    }

    /// Prune `_pylon_change_log` to at most `retain` events. Returns
    /// the number of rows deleted. Idempotent (returns 0 when the
    /// table is already at or below the retention bound). Runs from
    /// the persister thread on a low-frequency tick, so the
    /// `write_conn` acquisition queues behind active mutations
    /// rather than racing them.
    ///
    /// Semantics: the most-recent `retain` events are kept. A client
    /// whose cursor falls behind that window will see `ResyncRequired`
    /// from `/api/sync/pull` (existing behavior — the in-memory ring
    /// + persisted log are the storage for delta deliverability, and
    /// past the retention window the client must rehydrate from
    /// entity snapshots). Sizing `retain` trades disk for acceptable
    /// client-disconnect window.
    pub fn sqlite_change_log_prune(&self, retain: u64) -> Result<usize, RuntimeError> {
        let sb = self.sqlite_backend()?;
        let conn = sb.write_conn.lock().map_err(|e| RuntimeError {
            code: "SQLITE_LOCK_FAILED".into(),
            message: format!("write_conn lock poisoned: {e}"),
        })?;
        let max_seq: Option<i64> = conn
            .query_row("SELECT MAX(seq) FROM _pylon_change_log", [], |row| {
                row.get(0)
            })
            .ok();
        let max_seq = match max_seq {
            Some(m) => m,
            None => return Ok(0),
        };
        let cutoff = max_seq.saturating_sub(retain as i64);
        if cutoff <= 0 {
            return Ok(0);
        }
        conn.execute(
            "DELETE FROM _pylon_change_log WHERE seq <= ?1",
            rusqlite::params![cutoff],
        )
        .map_err(|e| RuntimeError {
            code: "SQLITE_CHANGE_LOG_PRUNE_FAILED".into(),
            message: format!("DELETE: {e}"),
        })
    }

    /// Return `true` when `_pylon_change_log` already has at least
    /// one event for the named entity. Used by the boot-time seed
    /// loop to decide PER ENTITY whether to skip seeding. A binary
    /// "any persisted event" gate (the original v0.3.224 shape)
    /// silently broke every entity added to the manifest after the
    /// database had any writes — the new entity's pre-existing rows
    /// never got Insert events into the log. Per-entity gating closes
    /// that gap.
    pub fn sqlite_change_log_has_entity(&self, entity: &str) -> bool {
        let sb = match self.sqlite_backend() {
            Ok(s) => s,
            Err(_) => return false,
        };
        let conn = match sb.write_conn.lock() {
            Ok(c) => c,
            Err(_) => return false,
        };
        conn.query_row(
            "SELECT 1 FROM _pylon_change_log WHERE entity = ?1 LIMIT 1",
            rusqlite::params![entity],
            |row| row.get::<_, i64>(0),
        )
        .is_ok()
    }

    /// Read the most recent `limit` events from `_pylon_change_log`,
    /// oldest-first. Used at boot to hydrate the in-memory ring
    /// buffer.
    pub fn sqlite_change_log_load_recent(&self, limit: usize) -> Vec<pylon_sync::ChangeEvent> {
        let sb = match self.sqlite_backend() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let conn = match sb.write_conn.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let mut stmt = match conn.prepare(
            "SELECT seq, entity, row_id, kind, data, prev_data, timestamp \
             FROM (SELECT seq, entity, row_id, kind, data, prev_data, timestamp \
                   FROM _pylon_change_log ORDER BY seq DESC LIMIT ?1) \
             ORDER BY seq ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(rusqlite::params![limit as i64], change_log_row_map) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.flatten().flatten().collect()
    }

    /// Read events with seq > `since` from `_pylon_change_log`,
    /// ordered ascending, capped at `limit`. Used by the pull
    /// fallback when the in-memory ring no longer covers the
    /// requested cursor range.
    /// Read persisted change-log events with `seq > since`, oldest first.
    /// Returns `None` on any storage failure (wrong backend, poisoned
    /// lock, prepare/query error) so the caller can distinguish "store
    /// momentarily unreadable" from "store confirmed empty" — only the
    /// latter should force a client resync. See [`pylon_sync::ChangeLogStore::pull_range`].
    pub fn sqlite_change_log_pull_range(
        &self,
        since: u64,
        limit: usize,
    ) -> Option<Vec<pylon_sync::ChangeEvent>> {
        let sb = self.sqlite_backend().ok()?;
        let conn = sb.write_conn.lock().ok()?;
        let mut stmt = conn
            .prepare(
                "SELECT seq, entity, row_id, kind, data, prev_data, timestamp \
                 FROM _pylon_change_log WHERE seq > ?1 ORDER BY seq ASC LIMIT ?2",
            )
            .ok()?;
        let rows = stmt
            .query_map(
                rusqlite::params![since as i64, limit as i64],
                change_log_row_map,
            )
            .ok()?;
        Some(rows.flatten().flatten().collect())
    }

    /// Reserve a chunk of `amount` seqs atomically — bumps the persisted
    /// high-water mark by `amount` and returns the new value. The
    /// caller treats the returned value as the upper bound of a
    /// reservation: seqs in `(returned - amount, returned]` are now
    /// safe to issue from in-memory state without further disk
    /// writes.
    ///
    /// This is the boot-time + background-thread persistence path.
    /// CRITICALLY: it does NOT run inside the mutation hot path —
    /// `change_log.append` returns from an in-memory atomic without
    /// ever touching SQLite. The bg thread holds its own
    /// write_conn lock long enough to do this one UPDATE and
    /// nothing else, so even on heavy mutation load there's no
    /// recursive-mutex deadlock (the bg thread just waits in line
    /// behind active mutation txs and gets its turn).
    ///
    /// Returns `None` on storage error; the SqliteSeqAllocator
    /// keeps using its existing reservation in that case (the next
    /// retry will eventually catch up).
    pub fn reserve_sqlite_change_seq(&self, amount: u64) -> Option<u64> {
        let sb = self.sqlite_backend().ok()?;
        let conn = sb.write_conn.lock().ok()?;
        conn.execute(
            "UPDATE _pylon_change_seq SET value = value + ? WHERE id = 1",
            rusqlite::params![amount as i64],
        )
        .ok()?;
        let v: i64 = conn
            .query_row(
                "SELECT value FROM _pylon_change_seq WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .ok()?;
        Some(v as u64)
    }

    /// Filesystem path to the SQLite database, if this runtime is file-backed.
    /// Returns `None` for in-memory runtimes AND Postgres runtimes (no local
    /// file). Used by the server bootstrap to derive companion paths
    /// (session store, change log persistence) without requiring the caller
    /// to pass them in.
    pub fn db_path(&self) -> Option<String> {
        if self.is_in_memory {
            return None;
        }
        let sb = match &self.backend {
            RuntimeBackend::Sqlite(sb) => sb,
            RuntimeBackend::Postgres(_) => return None,
        };
        let conn = sb.write_conn.lock().ok()?;
        conn.path().filter(|p| !p.is_empty()).map(String::from)
    }

    /// The shared Postgres connection pool, when this runtime is
    /// Postgres-backed. The auxiliary auth backends check connections out of
    /// this same pool instead of each opening a dedicated one, so an app's
    /// steady-state footprint is the pool size plus the scheduler-leader
    /// session, not the pool size plus one connection per auth backend.
    /// `None` for SQLite / in-memory runtimes.
    pub fn pg_shared_pool(&self) -> Option<std::sync::Arc<pylon_storage::pg_datastore::PgPool>> {
        match &self.backend {
            RuntimeBackend::Postgres(pg) => Some(pg.store.shared_pool()),
            RuntimeBackend::Sqlite(_) => None,
        }
    }

    /// Drop every row from every entity table. Intended for test harnesses
    /// that call `/api/__test__/reset` between cases; refuses to run on
    /// anything but an in-memory database.
    ///
    /// Does NOT drop the tables themselves — schema stays, indexes stay,
    /// triggers stay. Just truncates user data + the change log.
    pub fn reset_for_tests(&self) -> Result<(), RuntimeError> {
        if !self.is_in_memory() {
            return Err(RuntimeError {
                code: "RESET_REFUSED".into(),
                message: "reset_for_tests is only available on in-memory databases".into(),
            });
        }
        let conn = self.lock_write_conn()?;
        let entity_names: Vec<String> = self.entities.values().map(|e| e.name.clone()).collect();
        for name in entity_names {
            let sql = format!("DELETE FROM {}", quote_ident(&name));
            let _ = conn.execute(&sql, []);
            // Also clear any FTS5 shadow table if present.
            let fts_sql = format!("DELETE FROM {}", quote_ident(&format!("{name}_fts")));
            let _ = conn.execute(&fts_sql, []);
        }
        Ok(())
    }

    /// Create an in-memory SQLite-backed runtime (useful for tests and
    /// benchmarks). For Postgres-backed equivalents, use `open_postgres`
    /// with a test-cluster URL.
    pub fn in_memory(manifest: AppManifest) -> Result<Self, RuntimeError> {
        let conn = Connection::open_in_memory().map_err(|e| RuntimeError {
            code: "RUNTIME_OPEN_FAILED".into(),
            message: format!("Failed to open in-memory database: {e}"),
        })?;
        Self::from_connection(conn, manifest, true)
    }

    fn from_connection(
        conn: Connection,
        mut manifest: AppManifest,
        is_in_memory: bool,
    ) -> Result<Self, RuntimeError> {
        // Apply the production pragma set on the write connection. Fatal
        // on boot — a silent pragma miss here means subsequent ops have
        // no busy_timeout and would hang on any lock contention.
        tune_runtime_connection(&conn, is_in_memory).map_err(|e| RuntimeError {
            code: "PRAGMA_INIT_FAILED".into(),
            message: format!("pragma init on write connection: {e}"),
        })?;

        ensure_connection_entity(&mut manifest);
        ensure_cron_lease_entity(&mut manifest);
        force_vector_fields_server_only(&mut manifest);
        validate_manifest_org_roles(&manifest)?;
        validate_encrypted_fields(&manifest)?;
        // Encryption key + connections requirement check must run
        // BEFORE schema init — a manifest declaring connections
        // without a key shouldn't even reach the CREATE TABLE step.
        let encryption_key_early = load_encryption_key()?;
        validate_connection_encryption_present(&manifest, &encryption_key_early)?;
        // Build entity lookup map.
        let entities: HashMap<String, ManifestEntity> = manifest
            .entities
            .iter()
            .map(|e| (e.name.clone(), e.clone()))
            .collect();

        // Create tables for all entities.
        for entity in &manifest.entities {
            let fields: Vec<String> = entity
                .fields
                .iter()
                .map(|f| {
                    let col_type = match f.field_type.as_str() {
                        "int" => "INTEGER",
                        "float" => "REAL",
                        "bool" => "INTEGER",
                        _ => "TEXT",
                    };
                    let not_null = if f.optional { "" } else { " NOT NULL" };
                    let unique = if f.unique { " UNIQUE" } else { "" };
                    format!("{} {col_type}{not_null}{unique}", quote_ident(&f.name))
                })
                .collect();

            let mut cols = vec!["\"id\" TEXT PRIMARY KEY NOT NULL".to_string()];
            cols.extend(fields);
            let sql = format!(
                "CREATE TABLE IF NOT EXISTS {} ({})",
                quote_ident(&entity.name),
                cols.join(", ")
            );
            conn.execute(&sql, []).map_err(|e| RuntimeError {
                code: "SCHEMA_INIT_FAILED".into(),
                message: format!("Failed to create table {}: {e}", entity.name),
            })?;

            // Create indexes.
            for idx in &entity.indexes {
                let unique_kw = if idx.unique { "UNIQUE " } else { "" };
                let quoted_fields: Vec<String> =
                    idx.fields.iter().map(|f| quote_ident(f)).collect();
                let idx_sql = format!(
                    "CREATE {unique_kw}INDEX IF NOT EXISTS {} ON {} ({})",
                    quote_ident(&idx.name),
                    quote_ident(&entity.name),
                    quoted_fields.join(", ")
                );
                conn.execute(&idx_sql, []).ok();
            }

            // Create an FTS5 virtual table over all text-ish fields so clients
            // can do full-text search via the `$search` query operator.
            //
            // Fields that look like "string" / "richtext" / "text" are indexed.
            // The FTS table is a contentless external-content table pointed at
            // the entity table, so SQLite keeps it consistent via triggers we
            // install below.
            let text_fields: Vec<&str> = entity
                .fields
                .iter()
                .filter(|f| matches!(f.field_type.as_str(), "string" | "richtext" | "text"))
                .map(|f| f.name.as_str())
                .collect();
            let fts_name = format!("{}_fts", entity.name);
            // Columns of the FTS table as it exists on disk (empty = no
            // table). Migrations only ALTER the entity table, so when the
            // manifest's text-field set changes the FTS table keeps its old
            // shape — it must be rebuilt here or writes crash: a SQLite
            // table-rebuild migration drops the sync triggers with the
            // table, this startup path then recreates them against the
            // CURRENT field list, and the first INSERT dies with
            // "table <Entity>_fts has no column named <newField>"
            // (hit live: Batch gained `backgrounds`).
            let existing_fts_cols: Vec<String> = conn
                .prepare(&format!("PRAGMA table_info({})", quote_ident(&fts_name)))
                .ok()
                .map(|mut stmt| {
                    stmt.query_map([], |row| row.get::<_, String>(1))
                        .map(|rows| rows.filter_map(Result::ok).collect())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            let drop_fts = |conn: &Connection| {
                for suffix in ["ai", "ad", "au"] {
                    let _ = conn.execute(
                        &format!(
                            "DROP TRIGGER IF EXISTS {}",
                            quote_ident(&format!("{fts_name}_{suffix}"))
                        ),
                        [],
                    );
                }
                let _ = conn.execute(
                    &format!("DROP TABLE IF EXISTS {}", quote_ident(&fts_name)),
                    [],
                );
            };
            if text_fields.is_empty() {
                // Entity no longer has text columns — retire any leftover
                // index so $search doesn't join (and triggers don't write) a
                // stale table.
                if !existing_fts_cols.is_empty() {
                    drop_fts(&conn);
                }
            } else {
                let mut desired_sorted: Vec<&str> = text_fields.clone();
                desired_sorted.sort_unstable();
                let mut existing_sorted: Vec<&str> =
                    existing_fts_cols.iter().map(String::as_str).collect();
                existing_sorted.sort_unstable();
                let stale = !existing_fts_cols.is_empty() && existing_sorted != desired_sorted;
                if stale {
                    drop_fts(&conn);
                }
                // Backfill whenever the table is (re)created this boot: after
                // a stale rebuild the index must reflect existing rows, and on
                // an empty fresh table the rebuild is a no-op.
                let needs_backfill = stale || existing_fts_cols.is_empty();

                let quoted_cols: Vec<String> = text_fields.iter().map(|f| quote_ident(f)).collect();
                let fts_sql = format!(
                    "CREATE VIRTUAL TABLE IF NOT EXISTS {} USING fts5({}, content={}, content_rowid='rowid')",
                    quote_ident(&fts_name),
                    quoted_cols.join(", "),
                    quote_ident(&entity.name),
                );
                // FTS5 may not be compiled in; ignore errors so those builds
                // still work (queries using $search will return empty).
                let fts_ok = conn.execute(&fts_sql, []).is_ok();

                if fts_ok {
                    if needs_backfill {
                        // External-content rebuild: repopulate the index from
                        // the entity table it points at.
                        let rebuild = format!(
                            "INSERT INTO {ftb}({ftb}) VALUES('rebuild')",
                            ftb = quote_ident(&fts_name),
                        );
                        if let Err(e) = conn.execute(&rebuild, []) {
                            tracing::warn!(
                                "[fts] failed to rebuild index for {}: {e}",
                                entity.name
                            );
                        }
                    }
                    // Sync triggers: keep FTS index current on INSERT/UPDATE/DELETE.
                    //
                    // Subtle bug fixed: the trigger NAME must be built from
                    // the raw `fts_name` + suffix and THEN quoted once.
                    // Previously this code quoted `fts_name` first and then
                    // appended `_ai`/`_ad`/`_au` AFTER the closing quote,
                    // producing invalid SQL like `"foo_fts"_ai`. The
                    // `.ok()` after execute silently ate the error, so the
                    // triggers were never created and FTS stayed out of
                    // sync on writes.
                    let tbl = quote_ident(&entity.name);
                    let ftb = quote_ident(&fts_name);
                    let cols_list = quoted_cols.join(", ");
                    let new_list: Vec<String> = text_fields
                        .iter()
                        .map(|f| format!("new.{}", quote_ident(f)))
                        .collect();
                    let old_list: Vec<String> = text_fields
                        .iter()
                        .map(|f| format!("old.{}", quote_ident(f)))
                        .collect();

                    let trigger_ai = quote_ident(&format!("{}_ai", fts_name));
                    let trigger_ad = quote_ident(&format!("{}_ad", fts_name));
                    let trigger_au = quote_ident(&format!("{}_au", fts_name));

                    // Triggers are DROPped and recreated every boot (not
                    // IF NOT EXISTS): their column lists are baked into the
                    // trigger body, so a leftover trigger from an older
                    // manifest silently stops indexing new text fields.
                    let trigger_ins = format!(
                        "CREATE TRIGGER {trigger_ai} AFTER INSERT ON {tbl} BEGIN \
                         INSERT INTO {ftb}(rowid, {cols_list}) VALUES (new.rowid, {new_vals}); END",
                        new_vals = new_list.join(", "),
                    );
                    let trigger_del = format!(
                        "CREATE TRIGGER {trigger_ad} AFTER DELETE ON {tbl} BEGIN \
                         INSERT INTO {ftb}({ftb}, rowid, {cols_list}) VALUES('delete', old.rowid, {old_vals}); END",
                        old_vals = old_list.join(", "),
                    );
                    let trigger_upd = format!(
                        "CREATE TRIGGER {trigger_au} AFTER UPDATE ON {tbl} BEGIN \
                         INSERT INTO {ftb}({ftb}, rowid, {cols_list}) VALUES('delete', old.rowid, {old_vals}); \
                         INSERT INTO {ftb}(rowid, {cols_list}) VALUES (new.rowid, {new_vals}); END",
                        new_vals = new_list.join(", "),
                        old_vals = old_list.join(", "),
                    );
                    // Log failures instead of silently dropping — FTS going
                    // stale should be visible to operators.
                    for (label, trigger_name, sql) in [
                        ("ai", &trigger_ai, &trigger_ins),
                        ("ad", &trigger_ad, &trigger_del),
                        ("au", &trigger_au, &trigger_upd),
                    ] {
                        let _ = conn.execute(&format!("DROP TRIGGER IF EXISTS {trigger_name}"), []);
                        if let Err(e) = conn.execute(sql, []) {
                            tracing::warn!(
                                "[fts] failed to create {label} trigger for {}: {e}",
                                entity.name
                            );
                        }
                    }
                }
            }
        }

        // Open read-only connection pool for file-backed databases.
        // In-memory databases cannot share connections, so the pool stays empty
        // and reads fall back to the write connection.
        let db_path = conn.path().filter(|p| !p.is_empty()).map(|p| p.to_string());

        let read_pool = if let Some(ref path) = db_path {
            let mut pool = Vec::with_capacity(READ_POOL_SIZE);
            for _ in 0..READ_POOL_SIZE {
                let read_conn = Connection::open_with_flags(
                    path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .map_err(|e| RuntimeError {
                    code: "POOL_OPEN_FAILED".into(),
                    message: format!("Failed to open read connection: {e}"),
                })?;
                // Read-pool connection: fatal on tune failure too. If
                // the write conn tuned cleanly but a read-pool conn
                // doesn't, we'd ship a partially-tuned pool where some
                // queries hang on busy and others don't — clearer to
                // fail boot than to debug at 3 AM.
                tune_runtime_connection(&read_conn, false).map_err(|e| RuntimeError {
                    code: "PRAGMA_INIT_FAILED".into(),
                    message: format!("pragma init on read pool connection: {e}"),
                })?;
                pool.push(Mutex::new(read_conn));
            }
            pool
        } else {
            // In-memory DB — no separate read connections possible.
            Vec::new()
        };

        // Sidecar table for CRDT snapshots — created always so toggling
        // `crdt: true` on an entity post-deploy doesn't need a migration.
        crate::loro_store::ensure_sidecar(&conn).map_err(|e| RuntimeError {
            code: "CRDT_SIDECAR_FAILED".into(),
            message: format!("create CRDT sidecar table: {e}"),
        })?;

        let encrypted_fields = encryption_field_map(&entities);
        let encryption_key = encryption_key_early;
        Ok(Self {
            backend: RuntimeBackend::Sqlite(SqliteBackend {
                write_conn: Mutex::new(conn),
                read_pool,
                read_counter: AtomicUsize::new(0),
                crdt: crate::loro_store::LoroStore::new(),
            }),
            manifest: Arc::new(manifest),
            entities,
            is_in_memory,
            studio_config_path: RwLock::new(None),
            studio_entry_path: RwLock::new(None),
            encryption_key,
            encrypted_fields,
            connection_manager: std::sync::OnceLock::new(),
        })
    }

    /// Create the search index tables (`_facet_bitmap`, per-entity
    /// `_fts_<Entity>`, and a covering index for each declared
    /// sortable field) for every searchable entity in the manifest.
    ///
    /// Production deployments do this via the storage adapter's
    /// `apply_schema` / migration plan; that path also handles
    /// adding/removing the tables when a `search:` block is added or
    /// removed across deploys. This method is a quick path for tests
    /// and benchmarks that build a `Runtime::in_memory(...)` directly
    /// without going through the schema-plan pipeline.
    pub fn ensure_search_indexes(&self) -> Result<(), RuntimeError> {
        // Postgres: schema (FTS, facets) is owned by the storage adapter's
        // migration plan. Tests / benchmarks against Postgres must apply
        // the plan separately; this fast-path is a SQLite-only convenience.
        if matches!(self.backend, RuntimeBackend::Postgres(_)) {
            return Ok(());
        }
        let conn = self.lock_write_conn()?;
        conn.execute(pylon_storage::search::create_facet_table_sql(), [])
            .map_err(|e| RuntimeError {
                code: "FACET_TABLE_FAILED".into(),
                message: format!("create _facet_bitmap: {e}"),
            })?;
        for entity in &self.manifest.entities {
            if let Some(cfg) = &entity.search {
                if let Some(sql) = pylon_storage::search::create_fts_table_sql(&entity.name, cfg) {
                    conn.execute(&sql, []).map_err(|e| RuntimeError {
                        code: "FTS_TABLE_FAILED".into(),
                        message: format!("create FTS table for {}: {e}", entity.name),
                    })?;
                }
                for field in &cfg.sortable {
                    let idx_sql = format!(
                        "CREATE INDEX IF NOT EXISTS \"{}_sort_{field}\" ON \"{}\" (\"{field}\")",
                        entity.name, entity.name,
                    );
                    conn.execute(&idx_sql, []).map_err(|e| RuntimeError {
                        code: "SORT_INDEX_FAILED".into(),
                        message: format!("create sort index for {}.{field}: {e}", entity.name),
                    })?;
                }
            }
        }
        Ok(())
    }

    /// Return a reference to the app manifest.
    /// Encrypt every field in `data` declared `encrypted: true` for
    /// the given entity. Called on the WRITE side
    /// (insert/update) before the row hits the backend store.
    ///
    /// Returns an unmodified clone when the entity has no encrypted
    /// fields (the common case). On encryption failure (no key
    /// configured but encrypted fields present, or AEAD primitive
    /// rejection), returns a `RuntimeError` so the write surfaces
    /// `ENCRYPTION_FAILED` instead of silently writing plaintext.
    fn maybe_encrypt_row(
        &self,
        entity: &str,
        data: &serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        let fields = match self.encrypted_fields.get(entity) {
            Some(f) => f,
            None => return Ok(data.clone()),
        };
        let key = match &self.encryption_key {
            Some(k) => k,
            None => {
                return Err(RuntimeError {
                    code: "ENCRYPTION_NOT_CONFIGURED".into(),
                    message: format!(
                        "Entity \"{entity}\" has encrypted fields but PYLON_ENCRYPTION_KEY is not set."
                    ),
                });
            }
        };
        let mut out = data.clone();
        let field_refs: Vec<&str> = fields.iter().map(String::as_str).collect();
        encryption::encrypt_row_fields(key, entity, &mut out, &field_refs).map_err(|e| {
            RuntimeError {
                code: "ENCRYPTION_FAILED".into(),
                message: e.to_string(),
            }
        })?;
        Ok(out)
    }

    /// Read-boundary row normalization, called on every read path
    /// (get_by_id, list, list_after, lookup, query_filtered, transact
    /// pre-snapshots) for BOTH storage engines:
    ///
    ///  1. decrypt every field declared `encrypted: true` (plaintext
    ///     values without the `enc:v1:` prefix pass through, so rows
    ///     written before the field gained `encrypted: true` stay
    ///     readable);
    ///  2. parse `json`-typed fields from their stored serialized TEXT
    ///     form back into the real JSON value — callers (functions,
    ///     entity endpoints, serverData, sync events) never see the
    ///     string form.
    fn normalize_row_on_read(&self, entity: &str, row: &mut serde_json::Value) {
        if let (Some(fields), Some(key)) = (self.encrypted_fields.get(entity), &self.encryption_key)
        {
            let field_refs: Vec<&str> = fields.iter().map(String::as_str).collect();
            if let Err(e) = encryption::decrypt_row_fields(key, entity, row, &field_refs) {
                tracing::warn!(
                    "[encryption] Failed to decrypt encrypted fields on {entity}: {e}. \
                     Ciphertext returned as-is."
                );
            }
        }
        self.parse_json_fields_on_read(entity, row);
    }

    fn parse_json_fields_on_read(&self, entity: &str, row: &mut serde_json::Value) {
        if let Some(ent) = self.entities.get(entity) {
            parse_json_fields_in_row(ent, row);
            parse_vector_fields_in_row(ent, row);
        }
    }

    /// Crate-internal accessor for the loaded encryption key.
    /// Connections + future at-rest primitives consult it.
    pub(crate) fn encryption_key_for_test(&self) -> Option<encryption::EncryptionKey> {
        self.encryption_key.clone()
    }

    /// Lazy-init the shared ConnectionManager and return it.
    /// Codex P1 fix: the manager owns the state-token SQLite backend,
    /// which MUST be the same instance across HTTP routes and the
    /// function hook — otherwise tokens minted on auth-url don't
    /// exist when /callback runs.
    pub fn connection_manager(
        self: &std::sync::Arc<Self>,
    ) -> Option<std::sync::Arc<connections::ConnectionManager>> {
        self.connection_manager
            .get_or_init(|| {
                if self.manifest.connections.is_empty() {
                    return None;
                }
                let defs: Vec<connections::ConnectionDef> = self
                    .manifest
                    .connections
                    .iter()
                    .map(|c| connections::ConnectionDef {
                        name: c.name.clone(),
                        provider: c.provider.clone(),
                        scopes: c.scopes.clone(),
                    })
                    .collect();
                let state_backend: std::sync::Arc<dyn pylon_auth::OAuthStateBackend> =
                    match crate::oauth_backend::SqliteOAuthBackend::in_memory() {
                        Ok(b) => std::sync::Arc::new(b),
                        Err(e) => {
                            tracing::error!("[connections] failed to init state backend: {e}");
                            return None;
                        }
                    };
                Some(std::sync::Arc::new(connections::ConnectionManager::new(
                    defs,
                    self.encryption_key.clone(),
                    state_backend,
                )))
            })
            .clone()
    }

    pub fn manifest(&self) -> &AppManifest {
        self.manifest.as_ref()
    }

    /// Cheap shared handle to the manifest — an `Arc` refcount bump, not a deep
    /// clone. Use this where a caller needs to OWN an `Arc<AppManifest>` (e.g.
    /// the transactional-function store).
    pub fn manifest_arc(&self) -> Arc<AppManifest> {
        Arc::clone(&self.manifest)
    }

    /// Point the runtime at a `studio.config.json` written by the CLI.
    /// Subsequent `/studio` renders re-read this file from disk so dev
    /// edits to `studio.config.ts` take effect on refresh — no server
    /// restart needed.
    pub fn set_studio_config_path(&self, path: Option<PathBuf>) {
        if let Ok(mut guard) = self.studio_config_path.write() {
            *guard = path;
        }
    }

    /// Point the runtime at a bundled `studio.entry.js`. The HTTP layer
    /// serves this file at `/studio/extensions.js` (see
    /// `crates/runtime/src/server.rs`).
    pub fn set_studio_entry_path(&self, path: Option<PathBuf>) {
        if let Ok(mut guard) = self.studio_entry_path.write() {
            *guard = path;
        }
    }

    /// Load the current studio config from disk. Returns the default
    /// (empty) config when no file is configured or the file fails to
    /// parse — the Studio web shell falls back to sensible defaults
    /// in that case rather than rendering a blank screen.
    pub fn studio_config(&self) -> StudioConfig {
        let path = self.studio_config_path.read().ok().and_then(|g| g.clone());
        match path {
            Some(p) => match std::fs::read_to_string(&p) {
                Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
                Err(_) => StudioConfig::default(),
            },
            None => StudioConfig::default(),
        }
    }

    /// Read the bundled extensions JS, or `None` if not configured /
    /// missing on disk.
    pub fn studio_entry_bytes(&self) -> Option<Vec<u8>> {
        let path = self.studio_entry_path.read().ok().and_then(|g| g.clone())?;
        std::fs::read(&path).ok()
    }

    /// Expose the write connection mutex for transactional operations.
    /// SQLite-only — Postgres mode returns `NOT_SQLITE_BACKEND`. Callers
    /// that need a transaction on Postgres should use [`Runtime::transact_ops`]
    /// (via the `DataStore` trait), which routes to a real Postgres
    /// transaction inside `PostgresDataStore`.
    pub fn lock_conn_pub(&self) -> Result<std::sync::MutexGuard<'_, Connection>, RuntimeError> {
        self.lock_write_conn()
    }

    /// Return the number of read connections in the pool. Always 0 for
    /// in-memory SQLite (pool is empty by design) and for Postgres mode
    /// (the pool concept doesn't apply — `PostgresDataStore` manages its
    /// own connection internally).
    pub fn read_pool_size(&self) -> usize {
        match &self.backend {
            RuntimeBackend::Sqlite(sb) => sb.read_pool.len(),
            RuntimeBackend::Postgres(_) => 0,
        }
    }

    /// Return true if this runtime is backed by Postgres. Useful for
    /// SQLite-only fast-paths to early-exit cleanly.
    pub fn is_postgres(&self) -> bool {
        matches!(self.backend, RuntimeBackend::Postgres(_))
    }

    // -----------------------------------------------------------------------
    // CRDT helpers
    // -----------------------------------------------------------------------

    /// Map an entity's manifest fields → the [`pylon_crdt::CrdtField`] vec
    /// the LoroStore needs. Resolves each field's CRDT shape from the
    /// (type, annotation) pair via `pylon_crdt::field_kind`. Caches
    /// nothing yet — called per write, fine at our entity counts.
    pub(crate) fn crdt_fields_for(
        &self,
        ent: &ManifestEntity,
    ) -> Result<Vec<pylon_crdt::CrdtField>, RuntimeError> {
        let mut out = Vec::with_capacity(ent.fields.len());
        for f in &ent.fields {
            // Skip the implicit `id` column — it's the row key, not a
            // CRDT-managed value. SQLite's PRIMARY KEY constraint owns it.
            if f.name == "id" {
                continue;
            }
            // Skip vector fields entirely: embeddings are server-only,
            // and the LoroDoc is a client-syncing structure — including
            // them would ship multi-KB vectors in every binary CRDT
            // frame AND let a peer's merge projection overwrite the
            // packed column. The embedding lives in the SQL column
            // only; CRDT merges never touch it.
            if pylon_storage::vector::vector_dims(&f.field_type).is_some() {
                continue;
            }
            let kind = pylon_crdt::field_kind(&f.field_type, f.crdt).map_err(|e| RuntimeError {
                code: "INVALID_CRDT_FIELD".into(),
                message: format!(
                    "{}.{}: {e} (declared type={}, crdt={:?})",
                    ent.name, f.name, f.field_type, f.crdt
                ),
            })?;
            out.push(pylon_crdt::CrdtField {
                name: f.name.clone(),
                kind,
            });
        }
        Ok(out)
    }

    /// Borrow the CRDT store. SQLite-only — Postgres mode does not yet
    /// support per-row CRDT snapshots at the runtime layer (CRDT
    /// broadcasts degrade to JSON change events).
    ///
    /// # Panics
    /// Panics on Postgres backend. Call sites that may run under either
    /// backend should branch on `is_postgres()` first.
    pub fn crdt_store(&self) -> &crate::loro_store::LoroStore {
        match &self.backend {
            RuntimeBackend::Sqlite(sb) => &sb.crdt,
            RuntimeBackend::Postgres(_) => {
                panic!("crdt_store() called on Postgres-backed Runtime")
            }
        }
    }

    // -----------------------------------------------------------------------
    // CRUD operations
    // -----------------------------------------------------------------------

    /// Insert a new row. Returns the generated ID.
    ///
    /// For entities with `crdt: true` (the default) the LoroDoc snapshot
    /// + the SQLite materialized row are committed together in a single
    /// SQLite transaction so a crash between the two leaves neither.
    /// `crdt: false` entities skip the LoroDoc and use a direct write
    /// (legacy LWW path). Both produce the same on-disk row shape, so
    /// reads, indexes, FTS, and policies don't change between modes.
    pub fn insert(&self, entity: &str, data: &serde_json::Value) -> Result<String, RuntimeError> {
        // Apply manifest-declared field defaults BEFORE any storage
        // backend sees the row. `field.X().defaultNow()` and
        // `.default(value)` both flow through here — without this
        // pass, the framework would reject inserts that omit
        // non-optional fields with defaults, defeating the whole
        // point of declaring the default.
        let data_owned;
        let data = if entity_has_any_default(&self.manifest, entity) {
            data_owned = apply_field_defaults(&self.manifest, entity, data);
            &data_owned
        } else {
            data
        };
        // Encrypt encrypted-field values before the storage backend
        // sees the row. The `enc:v1:<nonce>:<ct>` strings persist
        // through CRDT projection, FTS indexing, JSON change events —
        // the wire never sees plaintext for these fields again.
        let encrypted_owned;
        let data = if self.encrypted_fields.contains_key(entity) {
            encrypted_owned = self.maybe_encrypt_row(entity, data)?;
            &encrypted_owned
        } else {
            data
        };
        validate_vector_fields(self.require_entity(entity)?, data)?;
        if let Some(pg) = self.pg_backend() {
            let ent = self.require_entity(entity)?;
            // `json` fields: pre-serialize to TEXT for the PG SQL
            // builders (pylon_storage never sees the manifest). The
            // CRDT patch below keeps the PARSED `data` — only the
            // materialized row stores the serialized form.
            let pg_ser;
            let pg_data = match serialize_json_fields_for_storage(ent, data) {
                Some(s) => {
                    pg_ser = s;
                    &pg_ser
                }
                None => data,
            };
            // Both CRDT-mode and non-CRDT writes go through one
            // transaction so the row, the FTS shadow, and (for CRDT)
            // the LoroDoc snapshot either all commit or all roll back.
            // Pre-fix this was three separate autocommits and any
            // failure between them desynced the layers.
            if ent.crdt {
                let crdt_fields = self.crdt_fields_for(ent)?;
                let id = resolve_or_generate_id(data)?;
                // Inject the resolved id so build_insert_sql reuses
                // it — keeps the snapshot key and the row id aligned.
                let mut row = pg_data.clone();
                if let Some(obj) = row.as_object_mut() {
                    obj.insert("id".into(), serde_json::Value::String(id.clone()));
                }
                let result = pg
                    .store
                    .with_transaction_raw(|tx| -> Result<(), RuntimeError> {
                        pg.crdt
                            .apply_patch(tx, entity, &id, &crdt_fields, data)
                            .map_err(|e| RuntimeError {
                                code: "CRDT_APPLY_FAILED".into(),
                                message: format!("crdt write {entity}/{id}: {e}"),
                            })?;
                        pylon_storage::pg_tx_store::tx_insert(tx, &self.manifest, entity, &row)
                            .map(|_| ())
                            .map_err(data_err_to_runtime)?;
                        pg.crdt.cache_after_commit(tx, entity, &id);
                        Ok(())
                    });
                if result.is_err() {
                    // Rollback drops the persisted snapshot, but the
                    // in-memory LoroDoc was mutated in-place by
                    // apply_patch. Evict it so the next access
                    // re-hydrates from disk (which is back in the
                    // pre-apply state). Without this, the cache would
                    // hold a doc ahead of the materialized row.
                    pg.crdt.evict(entity, &id);
                }
                result?;
                return Ok(id);
            }
            // Non-CRDT path: still one tx — the typed `DataStore::insert`
            // already wraps in `with_transaction` internally for FTS
            // atomicity, so we can delegate straight through.
            return pylon_http::DataStore::insert(&pg.store, entity, pg_data)
                .map_err(data_err_to_runtime);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_write_conn()?;

        let id = resolve_or_generate_id(data)?;

        let obj = data.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_DATA".into(),
            message: "Insert data must be a JSON object".into(),
        })?;

        // Validate columns up-front so we don't even open a transaction
        // for a patch that the SQL INSERT will reject.
        for key in obj.keys() {
            if key != "id" {
                validate_column_name(key, ent)?;
            }
        }

        // SQLite-only path past this point — Postgres dispatch happened
        // at the top of `insert()`. Hoist the backend handle here so we
        // can reach the LoroStore from inside the tx closure without
        // a second runtime branch on every iteration.
        let sb = self.sqlite_backend()?;

        // Atomic block — CRDT sidecar snapshot + materialized SQL row +
        // search-index maintenance all land together or none does. SQLite's
        // rollback journal makes this crash-safe end-to-end.
        with_write_tx(&conn, || {
            if ent.crdt {
                let crdt_fields = self.crdt_fields_for(ent)?;
                sb.crdt
                    .apply_patch(&conn, entity, &id, &crdt_fields, data)
                    .map_err(|e| RuntimeError {
                        code: "CRDT_APPLY_FAILED".into(),
                        message: format!("crdt write {entity}/{id}: {e}"),
                    })?;
            }

            let mut col_names = vec![quote_ident("id")];
            let mut placeholders = vec!["?1".to_string()];
            let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(id.clone())];

            let mut idx = 2;
            for (key, val) in obj {
                if key == "id" {
                    continue;
                }
                col_names.push(quote_ident(key));
                placeholders.push(format!("?{idx}"));
                values.push(json_to_sql_typed(ent, key, val));
                idx += 1;
            }

            let sql = format!(
                "INSERT INTO {} ({}) VALUES ({})",
                quote_ident(entity),
                col_names.join(", "),
                placeholders.join(", ")
            );

            let params: Vec<&dyn rusqlite::types::ToSql> =
                values.iter().map(|v| v.as_ref()).collect();
            conn.execute(&sql, params.as_slice()).map_err(|e| {
                let msg = e.to_string();
                // PK collision on a client-provided id (or, rare,
                // a generator collision) — surface as a typed code
                // so optimistic-mutation retry logic can tell this
                // apart from a generic write failure and either
                // re-issue with a fresh id or merge with the row
                // already present. Match `<entity>.id` specifically
                // so collisions on other UNIQUE columns (email,
                // slug, …) keep their generic INSERT_FAILED code.
                let code = if msg.contains(&format!("UNIQUE constraint failed: {entity}.id")) {
                    "OPTIMISTIC_ID_CONFLICT"
                } else {
                    "INSERT_FAILED"
                };
                RuntimeError {
                    code: code.into(),
                    message: format!("Insert into {entity} failed: {e}"),
                }
            })?;

            // Search-index maintenance lives inside the same tx so a
            // crash between the row insert and the FTS update can't leave
            // the search index inconsistent with the row table.
            if let Some(cfg) = ent.search.as_ref() {
                if !cfg.is_empty() {
                    pylon_storage::search_maintenance::apply_insert(&conn, entity, &id, data, cfg)
                        .map_err(|e| RuntimeError {
                            code: "SEARCH_MAINTENANCE_FAILED".into(),
                            message: format!("search index update on insert {entity}: {e}"),
                        })?;
                }
            }
            Ok(())
        })?;

        Ok(id)
    }

    /// Get a single row by ID.
    pub fn get_by_id(
        &self,
        entity: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            let mut row = pylon_http::DataStore::get_by_id(&pg.store, entity, id)
                .map_err(data_err_to_runtime)?;
            if let Some(r) = row.as_mut() {
                self.normalize_row_on_read(entity, r);
            }
            return Ok(row);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;

        let sql = format!("SELECT * FROM {} WHERE \"id\" = ?1", quote_ident(entity));
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare query: {e}"),
        })?;

        let fields = ent.fields.clone();

        let mut result = stmt
            .query_row(rusqlite::params![id], |row| Ok(row_to_json(row, &fields)))
            .ok();
        if let Some(r) = result.as_mut() {
            self.normalize_row_on_read(entity, r);
        }
        Ok(result)
    }

    /// List all rows for an entity.
    pub fn list(&self, entity: &str) -> Result<Vec<serde_json::Value>, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            let mut rows =
                pylon_http::DataStore::list(&pg.store, entity).map_err(data_err_to_runtime)?;
            for r in rows.iter_mut() {
                self.normalize_row_on_read(entity, r);
            }
            return Ok(rows);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;

        let sql = format!("SELECT * FROM {} ORDER BY \"id\"", quote_ident(entity));
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare query: {e}"),
        })?;

        let fields = ent.fields.clone();

        let rows = stmt
            .query_map([], |row| Ok(row_to_json(row, &fields)))
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Query failed: {e}"),
            })?;

        let mut result = Vec::new();
        for row in rows {
            if let Ok(mut val) = row {
                self.normalize_row_on_read(entity, &mut val);
                result.push(val);
            }
        }
        Ok(result)
    }

    /// List rows after a cursor ID (for cursor-based pagination).
    pub fn list_after(
        &self,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            let mut rows = pylon_http::DataStore::list_after(&pg.store, entity, after, limit)
                .map_err(data_err_to_runtime)?;
            for r in rows.iter_mut() {
                self.normalize_row_on_read(entity, r);
            }
            return Ok(rows);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;

        let fields = ent.fields.clone();
        let table = quote_ident(entity);

        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match after {
            Some(cursor) => (
                format!(
                    "SELECT * FROM {} WHERE \"id\" > ?1 ORDER BY \"id\" LIMIT ?2",
                    table
                ),
                vec![Box::new(cursor.to_string()), Box::new(limit as i64)],
            ),
            None => (
                format!("SELECT * FROM {} ORDER BY \"id\" LIMIT ?1", table),
                vec![Box::new(limit as i64)],
            ),
        };

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|v| v.as_ref()).collect();

        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare query: {e}"),
        })?;

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_json(row, &fields)))
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Query failed: {e}"),
            })?;

        let mut result = Vec::new();
        for row in rows {
            if let Ok(mut val) = row {
                self.normalize_row_on_read(entity, &mut val);
                result.push(val);
            }
        }
        Ok(result)
    }

    /// Rows in DESCENDING id order (newest first), starting strictly below
    /// `before` when given. The mirror of [`Runtime::list_after`].
    ///
    /// Backs `sync_limit`. Ids are monotonic, so "highest ids" is "most
    /// recent". Paged rather than one fetch because the cap counts rows the
    /// CALLER can see and visibility is decided per row — a single fetch would
    /// take the newest N across all tenants and then filter, starving a quiet
    /// tenant on a busy table.
    pub fn list_last(
        &self,
        entity: &str,
        before: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            let mut rows = pylon_http::DataStore::list_last(&pg.store, entity, before, limit)
                .map_err(data_err_to_runtime)?
                .unwrap_or_default();
            for r in rows.iter_mut() {
                self.normalize_row_on_read(entity, r);
            }
            return Ok(rows);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;
        let fields = ent.fields.clone();
        let table = quote_ident(entity);

        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match before {
            Some(cursor) => (
                format!(
                    "SELECT * FROM {} WHERE \"id\" < ?1 ORDER BY \"id\" DESC LIMIT ?2",
                    table
                ),
                vec![Box::new(cursor.to_string()), Box::new(limit as i64)],
            ),
            None => (
                format!("SELECT * FROM {} ORDER BY \"id\" DESC LIMIT ?1", table),
                vec![Box::new(limit as i64)],
            ),
        };
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|v| v.as_ref()).collect();

        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare query: {e}"),
        })?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_json(row, &fields)))
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Query failed: {e}"),
            })?;

        let mut result = Vec::new();
        for row in rows {
            if let Ok(mut val) = row {
                self.normalize_row_on_read(entity, &mut val);
                result.push(val);
            }
        }
        Ok(result)
    }

    /// Update a row by ID. Returns true if a row was found and updated.
    ///
    /// For entities with `crdt: true` (the default) the LoroDoc receives
    /// the patch first; the SQLite UPDATE writes the same fields so the
    /// materialized view stays in lockstep with the doc state.
    pub fn update(
        &self,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
    ) -> Result<bool, RuntimeError> {
        // Encrypt any encrypted-field patches before the backend
        // sees them. Partial updates (PATCH-style) only touch fields
        // the caller included — fields the caller omits stay as the
        // ciphertext that was already on disk.
        let encrypted_owned;
        let data = if self.encrypted_fields.contains_key(entity) {
            encrypted_owned = self.maybe_encrypt_row(entity, data)?;
            &encrypted_owned
        } else {
            data
        };
        validate_vector_fields(self.require_entity(entity)?, data)?;
        if let Some(pg) = self.pg_backend() {
            let ent = self.require_entity(entity)?;
            // `json` fields: serialized for the PG row write, parsed
            // for the CRDT patch — same split as `insert()`.
            let pg_ser;
            let pg_data = match serialize_json_fields_for_storage(ent, data) {
                Some(s) => {
                    pg_ser = s;
                    &pg_ser
                }
                None => data,
            };
            if ent.crdt {
                // CRDT mode: snapshot apply + materialized update +
                // FTS shadow rebuild all share one tx. Pre-fix the
                // snapshot landed in autocommit and the row write in
                // a separate one — a mid-write crash desynced them.
                //
                // The closure also FAILS the tx if `tx_update` returns
                // false (no row matched). Without that, the snapshot
                // would commit alone — orphaned state pointing at a
                // row that doesn't exist. Codex flagged this. On
                // rollback the runtime evicts the cached LoroDoc so
                // the next read re-hydrates from the (unchanged)
                // sidecar.
                let crdt_fields = self.crdt_fields_for(ent)?;
                let result = pg
                    .store
                    .with_transaction_raw(|tx| -> Result<bool, RuntimeError> {
                        pg.crdt
                            .apply_patch(tx, entity, id, &crdt_fields, data)
                            .map_err(|e| RuntimeError {
                                code: "CRDT_APPLY_FAILED".into(),
                                message: format!("crdt update {entity}/{id}: {e}"),
                            })?;
                        let updated = pylon_storage::pg_tx_store::tx_update(
                            tx,
                            &self.manifest,
                            entity,
                            id,
                            pg_data,
                        )
                        .map_err(data_err_to_runtime)?;
                        if !updated {
                            // Roll back via Err so the snapshot doesn't
                            // commit against a missing row.
                            return Err(RuntimeError {
                                code: "ENTITY_NOT_FOUND".into(),
                                message: format!(
                                    "Update on {entity}/{id} found no row — refusing to commit \
                                 a CRDT snapshot that would orphan."
                                ),
                            });
                        }
                        // Refresh the cache from the just-persisted
                        // snapshot so post-commit reads on this process
                        // skip the re-hydration round-trip.
                        pg.crdt.cache_after_commit(tx, entity, id);
                        Ok(updated)
                    });
                if result.is_err() {
                    pg.crdt.evict(entity, id);
                    // ENTITY_NOT_FOUND from the inner closure is the
                    // intended return for "no such row" — translate
                    // into Ok(false) so callers see the same shape
                    // the SQLite path returns. Real errors (CRDT
                    // apply failed, BEGIN/COMMIT failed) propagate.
                    if let Err(ref e) = result {
                        if e.code == "ENTITY_NOT_FOUND" {
                            return Ok(false);
                        }
                    }
                }
                return result;
            }
            return pylon_http::DataStore::update(&pg.store, entity, id, pg_data)
                .map_err(data_err_to_runtime);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_write_conn()?;

        let obj = data.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_DATA".into(),
            message: "Update data must be a JSON object".into(),
        })?;

        // Validate up-front and exit cheap if there's nothing to write.
        for key in obj.keys() {
            if key != "id" {
                validate_column_name(key, ent)?;
            }
        }
        let writable_keys: Vec<&String> = obj.keys().filter(|k| *k != "id").collect();
        if writable_keys.is_empty() {
            return Ok(false);
        }

        // SQLite-only path past this point — see note in `insert()`.
        let sb = self.sqlite_backend()?;

        // Atomic block — same shape as insert. CRDT snapshot, SQL UPDATE,
        // and FTS maintenance all commit together.
        let affected = with_write_tx(&conn, || -> Result<i64, RuntimeError> {
            if ent.crdt {
                let crdt_fields = self.crdt_fields_for(ent)?;
                sb.crdt
                    .apply_patch(&conn, entity, id, &crdt_fields, data)
                    .map_err(|e| RuntimeError {
                        code: "CRDT_APPLY_FAILED".into(),
                        message: format!("crdt write {entity}/{id}: {e}"),
                    })?;
            }

            let mut set_clauses = Vec::new();
            let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut idx = 1;
            for key in &writable_keys {
                set_clauses.push(format!("{} = ?{idx}", quote_ident(key)));
                values.push(json_to_sql_typed(ent, key, &obj[key.as_str()]));
                idx += 1;
            }

            // Capture pre-UPDATE row for search-maintenance diff INSIDE the
            // tx. Matches the contract of search_maintenance::apply_update
            // — old state must be read before the UPDATE lands.
            let searchable = ent.search.as_ref().map(|c| !c.is_empty()).unwrap_or(false);
            let old_row = if searchable {
                self.get_by_id_with_conn(&conn, entity, id)?
            } else {
                None
            };

            values.push(Box::new(id.to_string()));
            let sql = format!(
                "UPDATE {} SET {} WHERE \"id\" = ?{idx}",
                quote_ident(entity),
                set_clauses.join(", ")
            );

            let params: Vec<&dyn rusqlite::types::ToSql> =
                values.iter().map(|v| v.as_ref()).collect();
            let affected = conn
                .execute(&sql, params.as_slice())
                .map_err(|e| RuntimeError {
                    code: "UPDATE_FAILED".into(),
                    message: format!("Update {entity}/{id} failed: {e}"),
                })? as i64;

            if affected > 0 && searchable {
                if let (Some(cfg), Some(old)) = (ent.search.as_ref(), old_row) {
                    pylon_storage::search_maintenance::apply_update(
                        &conn, entity, id, &old, data, cfg,
                    )
                    .map_err(|e| RuntimeError {
                        code: "SEARCH_MAINTENANCE_FAILED".into(),
                        message: format!("search index update on update {entity}: {e}"),
                    })?;
                }
            }
            Ok(affected)
        })?;

        Ok(affected > 0)
    }

    /// Delete a row by ID. Returns true if a row was actually deleted.
    pub fn delete(&self, entity: &str, id: &str) -> Result<bool, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            let ent = self.require_entity(entity)?;
            if ent.crdt {
                // Sidecar delete + entity delete + FTS shadow delete
                // share one tx. Eviction of the in-memory cache runs
                // AFTER commit so a rolled-back delete leaves the
                // cache valid (the snapshot is still on disk).
                let result = pg
                    .store
                    .with_transaction_raw(|tx| -> Result<bool, RuntimeError> {
                        tx.execute(
                            "DELETE FROM _pylon_crdt_snapshots WHERE entity = $1 AND row_id = $2",
                            &[&entity, &id],
                        )
                        .map_err(|e| RuntimeError {
                            code: "CRDT_SIDECAR_DELETE_FAILED".into(),
                            message: format!("delete pg crdt snapshot {entity}/{id}: {e}"),
                        })?;
                        pylon_storage::pg_tx_store::tx_delete(tx, &self.manifest, entity, id)
                            .map_err(data_err_to_runtime)
                    });
                // Evict regardless of whether tx_delete found a row —
                // we issued the sidecar DELETE inside the same tx, so
                // any cached doc is now stale even if the entity row
                // was already gone (orphan sidecar case codex flagged).
                // Only skip eviction if the WHOLE tx rolled back.
                if result.is_ok() {
                    pg.crdt.evict(entity, id);
                }
                return result;
            }
            return pylon_http::DataStore::delete(&pg.store, entity, id)
                .map_err(data_err_to_runtime);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_write_conn()?;

        // Apply search-maintenance BEFORE the DELETE — we still need
        // the row's facet values to clear the bitmap bits.
        let searchable = ent.search.as_ref().map(|c| !c.is_empty()).unwrap_or(false);
        if searchable {
            if let (Some(cfg), Ok(Some(row))) = (
                ent.search.as_ref(),
                self.get_by_id_with_conn(&conn, entity, id),
            ) {
                pylon_storage::search_maintenance::apply_delete(&conn, entity, id, &row, cfg)
                    .map_err(|e| RuntimeError {
                        code: "SEARCH_MAINTENANCE_FAILED".into(),
                        message: format!("search index update on delete {entity}: {e}"),
                    })?;
            }
        }

        let sql = format!("DELETE FROM {} WHERE \"id\" = ?1", quote_ident(entity));
        let affected = conn
            .execute(&sql, rusqlite::params![id])
            .map_err(|e| RuntimeError {
                code: "DELETE_FAILED".into(),
                message: format!("Delete {entity}/{id} failed: {e}"),
            })?;

        Ok(affected > 0)
    }

    /// Lookup a single row by a field value (e.g., email).
    ///
    /// Encrypted fields are NOT queryable here — each write produces
    /// fresh ciphertext (random nonce), so a lookup by an encrypted
    /// field's plaintext value never matches anything. The caller
    /// will get `Ok(None)` even when the row exists. Document this
    /// constraint in your handler if it surprises a user.
    pub fn lookup(
        &self,
        entity: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            let mut row = pylon_http::DataStore::lookup(&pg.store, entity, field, value)
                .map_err(data_err_to_runtime)?;
            if let Some(r) = row.as_mut() {
                self.normalize_row_on_read(entity, r);
            }
            return Ok(row);
        }
        let ent = self.require_entity(entity)?;
        validate_column_name(field, ent)?;
        let conn = self.lock_read_conn()?;

        let sql = format!(
            "SELECT * FROM {} WHERE {} = ?1 LIMIT 1",
            quote_ident(entity),
            quote_ident(field)
        );
        let fields = ent.fields.clone();

        let mut result = conn.prepare_cached(&sql).ok().and_then(|mut stmt| {
            stmt.query_row(rusqlite::params![value], |row| {
                Ok(row_to_json(row, &fields))
            })
            .ok()
        });
        if let Some(r) = result.as_mut() {
            self.normalize_row_on_read(entity, r);
        }
        Ok(result)
    }

    /// Link two entities by setting a foreign-key field.
    pub fn link(
        &self,
        entity: &str,
        id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, RuntimeError> {
        let ent = self.require_entity(entity)?;

        // Find the relation definition to determine which field to set.
        let rel = ent
            .relations
            .iter()
            .find(|r| r.name == relation)
            .ok_or_else(|| RuntimeError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found on entity \"{entity}\""),
            })?;

        let data = serde_json::json!({ rel.field.clone(): target_id });
        self.update(entity, id, &data)
    }

    /// Unlink a relation by setting the foreign-key field to null.
    pub fn unlink(&self, entity: &str, id: &str, relation: &str) -> Result<bool, RuntimeError> {
        let ent = self.require_entity(entity)?;

        let rel = ent
            .relations
            .iter()
            .find(|r| r.name == relation)
            .ok_or_else(|| RuntimeError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found on entity \"{entity}\""),
            })?;

        let data = serde_json::json!({ rel.field.clone(): null });
        self.update(entity, id, &data)
    }

    /// Execute a filtered query with operators ($not, $gt, $in, $like, $order, $limit).
    pub fn query_filtered(
        &self,
        entity: &str,
        filter: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            let mut rows = pylon_http::DataStore::query_filtered(&pg.store, entity, filter)
                .map_err(data_err_to_runtime)?;
            // Decrypt `field.encrypted()` columns, same as get_by_id/list/lookup
            // — a server-side ctx.db.query() must see plaintext, not ciphertext.
            for r in rows.iter_mut() {
                self.normalize_row_on_read(entity, r);
            }
            return Ok(rows);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;

        let fields = ent.fields.clone();
        let obj = filter
            .as_object()
            .unwrap_or(&serde_json::Map::new())
            .clone();

        let mut where_clauses = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut order_clause = String::new();
        let mut limit_clause = String::new();
        // Captured raw, then bounded after the loop (see query_max_limit):
        // a client $limit is clamped to the cap and a missing one defaults
        // to it, so an uncapped SELECT can't materialize a whole table.
        let mut client_limit: Option<u64> = None;
        let mut client_offset: Option<u64> = None;
        let mut join_clause = String::new();
        let mut fts_order = false;
        let mut idx = 1;

        for (key, val) in &obj {
            match key.as_str() {
                "$order" => {
                    if let Some(order_obj) = val.as_object() {
                        let mut parts: Vec<String> = Vec::new();
                        for (col, dir) in order_obj {
                            validate_column_name(col, ent)?;
                            let d = match dir.as_str().unwrap_or("asc") {
                                "desc" | "DESC" => "DESC",
                                _ => "ASC",
                            };
                            parts.push(format!("{} {d}", quote_ident(col)));
                        }
                        if !parts.is_empty() {
                            order_clause = format!(" ORDER BY {}", parts.join(", "));
                        }
                    }
                }
                "$limit" => {
                    if let Some(n) = val.as_u64() {
                        client_limit = Some(n);
                    }
                }
                "$offset" => {
                    if let Some(n) = val.as_u64() {
                        client_offset = Some(n);
                    }
                }
                "$search" => {
                    if let Some(q) = val.as_str() {
                        // Join against the entity's FTS5 virtual table.
                        let fts = format!("{}_fts", entity);
                        join_clause = format!(
                            " JOIN {fts} ON {ent}.rowid = {fts}.rowid",
                            fts = quote_ident(&fts),
                            ent = quote_ident(entity),
                        );
                        where_clauses.push(format!("{} MATCH ?{idx}", quote_ident(&fts)));
                        values.push(Box::new(q.to_string()));
                        fts_order = true;
                        idx += 1;
                    }
                }
                _ => {
                    validate_column_name(key, ent)?;
                    let quoted_key = quote_ident(key);

                    // A json-typed column filtered with a plain object
                    // (no $-operators) is a WHOLE-VALUE equality match
                    // on the serialized form — otherwise the object
                    // would be misread as an operator map and silently
                    // match everything.
                    if entity_field_is_json(ent, key)
                        && val
                            .as_object()
                            .is_some_and(|o| !o.keys().any(|k| k.starts_with('$')))
                    {
                        where_clauses.push(format!("{quoted_key} = ?{idx}"));
                        values.push(json_to_sql_typed(ent, key, val));
                        idx += 1;
                    } else if let Some(op_obj) = val.as_object() {
                        for (op, op_val) in op_obj {
                            match op.as_str() {
                                "$not" => {
                                    where_clauses.push(format!("{quoted_key} != ?{idx}"));
                                    values.push(json_to_sql_typed(ent, key, op_val));
                                    idx += 1;
                                }
                                "$gt" => {
                                    where_clauses.push(format!("{quoted_key} > ?{idx}"));
                                    values.push(json_to_sql_typed(ent, key, op_val));
                                    idx += 1;
                                }
                                "$gte" => {
                                    where_clauses.push(format!("{quoted_key} >= ?{idx}"));
                                    values.push(json_to_sql_typed(ent, key, op_val));
                                    idx += 1;
                                }
                                "$lt" => {
                                    where_clauses.push(format!("{quoted_key} < ?{idx}"));
                                    values.push(json_to_sql_typed(ent, key, op_val));
                                    idx += 1;
                                }
                                "$lte" => {
                                    where_clauses.push(format!("{quoted_key} <= ?{idx}"));
                                    values.push(json_to_sql_typed(ent, key, op_val));
                                    idx += 1;
                                }
                                "$like" => {
                                    where_clauses.push(format!("{quoted_key} LIKE ?{idx}"));
                                    let pattern = format!("%{}%", op_val.as_str().unwrap_or(""));
                                    values.push(Box::new(pattern));
                                    idx += 1;
                                }
                                "$in" => {
                                    if let Some(arr) = op_val.as_array() {
                                        if arr.is_empty() {
                                            // Empty $in matches nothing.
                                            // Previously SQLite SKIPPED the
                                            // predicate (returning ALL rows)
                                            // while PG short-circuited to
                                            // FALSE — a real cross-backend
                                            // drift bug codex caught. Both
                                            // now emit `0` (false) so empty
                                            // $in returns an empty set.
                                            where_clauses.push("0".into());
                                        } else {
                                            let placeholders: Vec<String> = arr
                                                .iter()
                                                .map(|v| {
                                                    let p = format!("?{idx}");
                                                    values.push(json_to_sql_typed(ent, key, v));
                                                    idx += 1;
                                                    p
                                                })
                                                .collect();
                                            where_clauses.push(format!(
                                                "{quoted_key} IN ({})",
                                                placeholders.join(", ")
                                            ));
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else {
                        // Simple equality.
                        where_clauses.push(format!("{quoted_key} = ?{idx}"));
                        values.push(json_to_sql_typed(ent, key, val));
                        idx += 1;
                    }
                }
            }
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };

        if order_clause.is_empty() {
            order_clause = if fts_order {
                // FTS joins default-order by bm25 relevance.
                " ORDER BY bm25(".to_string() + &quote_ident(&format!("{}_fts", entity)) + ")"
            } else {
                format!(" ORDER BY {}.\"id\"", quote_ident(entity))
            };
        }

        // Bound the result set: clamp a client $limit and default a missing
        // one to the cap, so `{}` can't stream the whole table into memory.
        let effective_limit = pylon_kernel::util::effective_query_limit(client_limit);
        limit_clause = match client_offset {
            Some(off) => format!(" LIMIT {effective_limit} OFFSET {off}"),
            None => format!(" LIMIT {effective_limit}"),
        };
        let select_prefix = format!("{}.*", quote_ident(entity));
        let sql = format!(
            "SELECT {} FROM {}{}{}{}{}",
            select_prefix,
            quote_ident(entity),
            join_clause,
            where_sql,
            order_clause,
            limit_clause
        );
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|v| v.as_ref()).collect();

        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare filtered query: {e}"),
        })?;

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_json(row, &fields)))
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Filtered query failed: {e}"),
            })?;

        let mut result = Vec::new();
        for row in rows {
            if let Ok(mut val) = row {
                // Decrypt `field.encrypted()` columns — see the PG branch above
                // and get_by_id/list/lookup. normalize_row_on_read self-guards, so
                // entities with no encrypted fields pay nothing.
                self.normalize_row_on_read(entity, &mut val);
                result.push(val);
            }
        }
        Ok(result)
    }

    /// Execute a graph-style query.
    ///
    /// Input: `{ "User": { "where": { "email": "..." }, "include": { "posts": {} } } }`
    /// Returns nested results following relations.
    pub fn query_graph(
        &self,
        query: &serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        let obj = query.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_QUERY".into(),
            message: "Graph query must be a JSON object".into(),
        })?;

        let mut results = serde_json::Map::new();

        for (entity_name, query_opts) in obj {
            let _ent = self.require_entity(entity_name)?;

            // Apply where clause if present.
            let filter = query_opts
                .get("where")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            let rows = self.query_filtered(entity_name, &filter)?;

            // Apply includes (relations) if present.
            let rows = if let Some(include) = query_opts.get("include").and_then(|v| v.as_object())
            {
                // Internal invariant: if query_filtered succeeded above, the
                // entity must exist. Previously this used .unwrap() which
                // would panic if the invariant broke — a panic inside the
                // handler path poisons the connection mutex and takes down
                // all subsequent reads. Fail the request cleanly instead.
                let ent = self.entities.get(entity_name).ok_or_else(|| RuntimeError {
                    code: "INVARIANT_BROKEN".into(),
                    message: format!(
                        "entity \"{entity_name}\" missing from registry during include expansion"
                    ),
                })?;
                // Expand each relation with ONE batched child query instead of
                // a separate query PER PARENT ROW (the old N+1). For M parents
                // we collect the M foreign-key values, fetch every matching
                // child in a single `$in` query, bucket them by the join key,
                // then hand each parent its bucket. M×K queries become K.
                //
                // Behavior note: the batched child query shares the standard
                // query LIMIT across ALL parents (vs. a per-parent limit
                // before), so an include that fans out to more than
                // `query_max_limit` total children is now capped in aggregate.
                // That bounds a graph query's response rather than letting it
                // balloon, and only affects pathological fan-outs.
                let mut rows = rows;
                for (rel_name, _sub_query) in include {
                    let Some(rel) = ent.relations.iter().find(|r| r.name == *rel_name) else {
                        continue;
                    };
                    // Distinct foreign keys across all parents (first-seen
                    // order — keeps the `$in` list deterministic).
                    let mut fks: Vec<String> = Vec::new();
                    let mut seen: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    for row in rows.iter() {
                        if let Some(fk) = row.get(&rel.field).and_then(|v| v.as_str()) {
                            if seen.insert(fk.to_string()) {
                                fks.push(fk.to_string());
                            }
                        }
                    }
                    if fks.is_empty() {
                        continue;
                    }

                    if rel.many {
                        // One-to-many: children whose `rel.field` matches the
                        // parent's `rel.field`. Bucket by that same join key.
                        let filter = serde_json::json!({ &rel.field: { "$in": &fks } });
                        let related = match self.query_filtered(&rel.target, &filter) {
                            Ok(r) => r,
                            Err(_) => continue, // matches old: a failed child query left the field unset
                        };
                        let mut buckets: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
                        for child in related {
                            if let Some(k) = child
                                .get(&rel.field)
                                .and_then(|v| v.as_str())
                                .map(String::from)
                            {
                                buckets.entry(k).or_default().push(child);
                            }
                        }
                        for row in rows.iter_mut() {
                            if let Some(fk) = row
                                .get(&rel.field)
                                .and_then(|v| v.as_str())
                                .map(String::from)
                            {
                                let mine = buckets.get(&fk).cloned().unwrap_or_default();
                                row[rel_name.as_str()] = serde_json::json!(mine);
                            }
                        }
                    } else {
                        // Many-to-one / one-to-one: child keyed by its `id`.
                        let filter = serde_json::json!({ "id": { "$in": &fks } });
                        let related = match self.query_filtered(&rel.target, &filter) {
                            Ok(r) => r,
                            Err(_) => continue,
                        };
                        let mut by_id: HashMap<String, serde_json::Value> = HashMap::new();
                        for child in related {
                            if let Some(id) =
                                child.get("id").and_then(|v| v.as_str()).map(String::from)
                            {
                                by_id.insert(id, child);
                            }
                        }
                        for row in rows.iter_mut() {
                            if let Some(fk) = row.get(&rel.field).and_then(|v| v.as_str()) {
                                if let Some(child) = by_id.get(fk) {
                                    // Only assign on a match — same as the old
                                    // get_by_id, which left the field unset on None.
                                    row[rel_name.as_str()] = child.clone();
                                }
                            }
                        }
                    }
                }
                rows
            } else {
                rows
            };

            // Apply limit if present.
            let rows = if let Some(limit) = query_opts.get("limit").and_then(|v| v.as_u64()) {
                rows.into_iter().take(limit as usize).collect()
            } else {
                rows
            };

            results.insert(entity_name.clone(), serde_json::json!(rows));
        }

        Ok(serde_json::Value::Object(results))
    }

    // -----------------------------------------------------------------------
    // Transaction-safe variants (use a pre-held connection guard)
    // -----------------------------------------------------------------------

    /// Insert using an already-locked connection (for transactions).
    pub fn insert_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        data: &serde_json::Value,
    ) -> Result<String, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let id = resolve_or_generate_id(data)?;
        // Encrypt encrypted-field values before this connection sees
        // the row. Codex P1: the *_with_conn family is the per-
        // transaction store path used by ALL action handlers (TxStore,
        // PgBufferedTxStore). Without this, plaintext lands in the DB
        // for every action-handler write.
        let encrypted_owned;
        let data = if self.encrypted_fields.contains_key(entity) {
            encrypted_owned = self.maybe_encrypt_row(entity, data)?;
            &encrypted_owned
        } else {
            data
        };
        validate_vector_fields(ent, data)?;

        // Seed the per-row LoroDoc for CRDT entities, in the SAME transaction
        // as the SQL insert, so the CRDT sidecar and the materialized columns
        // agree from the very first write. `Runtime::insert` already does this;
        // the `*_with_conn` family — the transaction-bound store path behind
        // EVERY action/mutation `ctx.db.insert` — silently didn't. The gap was
        // invisible until a row inserted by a server function later received a
        // `/api/crdt/<entity>/<id>` update: with an empty LoroDoc, the
        // post-merge projection wrote NULLs over every non-CRDT column
        // (orgId, foreign keys, …), failing NOT NULL constraints and clobbering
        // data. Seeding here makes server-created rows first-class CRDT rows,
        // identical to client-sync inserts. SQLite-only path (rusqlite
        // Connection), so `crdt_store()` never hits its Postgres panic.
        if ent.crdt {
            let crdt_fields = self.crdt_fields_for(ent)?;
            self.crdt_store()
                .apply_patch(conn, entity, &id, &crdt_fields, data)
                .map_err(|e| RuntimeError {
                    code: "CRDT_APPLY_FAILED".into(),
                    message: format!("crdt seed {entity}/{id}: {e}"),
                })?;
        }

        let obj = data.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_DATA".into(),
            message: "Insert data must be a JSON object".into(),
        })?;

        let mut col_names = vec![quote_ident("id")];
        let mut placeholders = vec!["?1".to_string()];
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(id.clone())];
        let mut idx = 2;
        for (key, val) in obj {
            if key == "id" {
                continue;
            }
            validate_column_name(key, ent)?;
            col_names.push(quote_ident(key));
            placeholders.push(format!("?{idx}"));
            values.push(json_to_sql_typed(ent, key, val));
            idx += 1;
        }

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_ident(entity),
            col_names.join(", "),
            placeholders.join(", ")
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        conn.execute(&sql, params.as_slice()).map_err(|e| {
            let msg = e.to_string();
            let code = if msg.contains("UNIQUE constraint failed") {
                "OPTIMISTIC_ID_CONFLICT"
            } else {
                "INSERT_FAILED"
            };
            RuntimeError {
                code: code.into(),
                message: format!("Insert into {entity} failed: {e}"),
            }
        })?;

        // Faceted-search maintenance in the same transaction. Skipped
        // for entities that don't declare `search:` in their schema.
        if let Some(cfg) = ent.search.as_ref() {
            if !cfg.is_empty() {
                pylon_storage::search_maintenance::apply_insert(conn, entity, &id, data, cfg)
                    .map_err(|e| RuntimeError {
                        code: "SEARCH_MAINTENANCE_FAILED".into(),
                        message: format!("search index update on insert {entity}: {e}"),
                    })?;
            }
        }

        Ok(id)
    }

    /// Update using an already-locked connection (for transactions).
    pub fn update_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        id: &str,
        data: &serde_json::Value,
    ) -> Result<bool, RuntimeError> {
        let ent = self.require_entity(entity)?;
        // Encrypt before this connection sees the patch — same
        // rationale as insert_with_conn.
        let encrypted_owned;
        let data = if self.encrypted_fields.contains_key(entity) {
            encrypted_owned = self.maybe_encrypt_row(entity, data)?;
            &encrypted_owned
        } else {
            data
        };
        validate_vector_fields(ent, data)?;
        let obj = data.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_DATA".into(),
            message: "Update data must be a JSON object".into(),
        })?;

        let mut set_clauses = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut idx = 1;
        for (key, val) in obj {
            if key == "id" {
                continue;
            }
            validate_column_name(key, ent)?;
            set_clauses.push(format!("{} = ?{idx}", quote_ident(key)));
            values.push(json_to_sql_typed(ent, key, val));
            idx += 1;
        }
        if set_clauses.is_empty() {
            return Ok(false);
        }

        // Capture the pre-UPDATE row if we need to diff facet values.
        // Read happens before the UPDATE so apply_update sees the OLD
        // state of any facet field. Cheap — single-row lookup on the
        // `id` primary-key index.
        let searchable = ent.search.as_ref().map(|c| !c.is_empty()).unwrap_or(false);
        let old_row = if searchable {
            self.get_by_id_with_conn(conn, entity, id)?
        } else {
            None
        };

        values.push(Box::new(id.to_string()));
        let sql = format!(
            "UPDATE {} SET {} WHERE \"id\" = ?{idx}",
            quote_ident(entity),
            set_clauses.join(", ")
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        let affected = conn
            .execute(&sql, params.as_slice())
            .map_err(|e| RuntimeError {
                code: "UPDATE_FAILED".into(),
                message: format!("Update {entity}/{id} failed: {e}"),
            })?;

        if affected > 0 && searchable {
            if let (Some(cfg), Some(old)) = (ent.search.as_ref(), old_row) {
                pylon_storage::search_maintenance::apply_update(conn, entity, id, &old, data, cfg)
                    .map_err(|e| RuntimeError {
                        code: "SEARCH_MAINTENANCE_FAILED".into(),
                        message: format!("search index update on update {entity}: {e}"),
                    })?;
            }
        }

        Ok(affected > 0)
    }

    /// Delete using an already-locked connection (for transactions).
    pub fn delete_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        id: &str,
    ) -> Result<bool, RuntimeError> {
        let ent = self.require_entity(entity)?;

        // Apply search maintenance BEFORE the DELETE so we still have
        // the row's facet values to diff against.
        let searchable = ent.search.as_ref().map(|c| !c.is_empty()).unwrap_or(false);
        if searchable {
            if let (Some(cfg), Ok(Some(row))) = (
                ent.search.as_ref(),
                self.get_by_id_with_conn(conn, entity, id),
            ) {
                pylon_storage::search_maintenance::apply_delete(conn, entity, id, &row, cfg)
                    .map_err(|e| RuntimeError {
                        code: "SEARCH_MAINTENANCE_FAILED".into(),
                        message: format!("search index update on delete {entity}: {e}"),
                    })?;
            }
        }

        let sql = format!("DELETE FROM {} WHERE \"id\" = ?1", quote_ident(entity));
        let affected = conn
            .execute(&sql, rusqlite::params![id])
            .map_err(|e| RuntimeError {
                code: "DELETE_FAILED".into(),
                message: format!("Delete {entity}/{id} failed: {e}"),
            })?;
        Ok(affected > 0)
    }

    /// Read a row by id using a pre-held connection (for transactions).
    pub fn get_by_id_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        id: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let sql = format!("SELECT * FROM {} WHERE \"id\" = ?1", quote_ident(entity));
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare query: {e}"),
        })?;
        let fields = ent.fields.clone();
        let mut row = stmt
            .query_row(rusqlite::params![id], |row| Ok(row_to_json(row, &fields)))
            .ok();
        if let Some(r) = row.as_mut() {
            self.normalize_row_on_read(entity, r);
        }
        Ok(row)
    }

    /// List rows using a pre-held connection (for transactions).
    pub fn list_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
    ) -> Result<Vec<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let sql = format!("SELECT * FROM {} ORDER BY \"id\"", quote_ident(entity));
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare query: {e}"),
        })?;
        let fields = ent.fields.clone();
        let rows = stmt
            .query_map([], |row| Ok(row_to_json(row, &fields)))
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Query failed: {e}"),
            })?;
        let mut out: Vec<serde_json::Value> = rows.flatten().collect();
        for r in out.iter_mut() {
            self.normalize_row_on_read(entity, r);
        }
        Ok(out)
    }

    /// List after cursor using a pre-held connection (for transactions).
    pub fn list_after_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let fields = ent.fields.clone();
        let table = quote_ident(entity);
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match after {
            Some(cursor) => (
                format!("SELECT * FROM {table} WHERE \"id\" > ?1 ORDER BY \"id\" LIMIT ?2"),
                vec![Box::new(cursor.to_string()), Box::new(limit as i64)],
            ),
            None => (
                format!("SELECT * FROM {table} ORDER BY \"id\" LIMIT ?1"),
                vec![Box::new(limit as i64)],
            ),
        };
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|v| v.as_ref()).collect();
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare: {e}"),
        })?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_json(row, &fields)))
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Query failed: {e}"),
            })?;
        let mut out: Vec<serde_json::Value> = rows.flatten().collect();
        for r in out.iter_mut() {
            self.normalize_row_on_read(entity, r);
        }
        Ok(out)
    }

    /// Lookup by field using a pre-held connection (for transactions).
    pub fn lookup_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        field: &str,
        value: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        validate_column_name(field, ent)?;
        let sql = format!(
            "SELECT * FROM {} WHERE {} = ?1 LIMIT 1",
            quote_ident(entity),
            quote_ident(field)
        );
        let fields = ent.fields.clone();
        let mut row = conn.prepare_cached(&sql).ok().and_then(|mut stmt| {
            stmt.query_row(rusqlite::params![value], |row| {
                Ok(row_to_json(row, &fields))
            })
            .ok()
        });
        if let Some(r) = row.as_mut() {
            self.normalize_row_on_read(entity, r);
        }
        Ok(row)
    }

    /// Link relation using a pre-held connection (for transactions).
    pub fn link_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let rel = ent
            .relations
            .iter()
            .find(|r| r.name == relation)
            .ok_or_else(|| RuntimeError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found on \"{entity}\""),
            })?;
        let data = serde_json::json!({ rel.field.clone(): target_id });
        self.update_with_conn(conn, entity, id, &data)
    }

    /// Unlink relation using a pre-held connection (for transactions).
    pub fn unlink_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        id: &str,
        relation: &str,
    ) -> Result<bool, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let rel = ent
            .relations
            .iter()
            .find(|r| r.name == relation)
            .ok_or_else(|| RuntimeError {
                code: "RELATION_NOT_FOUND".into(),
                message: format!("Relation \"{relation}\" not found on \"{entity}\""),
            })?;
        let data = serde_json::json!({ rel.field.clone(): serde_json::Value::Null });
        self.update_with_conn(conn, entity, id, &data)
    }

    /// Query with filters using a pre-held connection (for transactions).
    ///
    /// Shares the filter-building logic with [`query_filtered`] by executing
    /// against the provided connection rather than acquiring one.
    pub fn query_filtered_with_conn(
        &self,
        conn: &Connection,
        entity: &str,
        filter: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>, RuntimeError> {
        let ent = self.require_entity(entity)?;
        let fields = ent.fields.clone();
        let empty = serde_json::Map::new();
        let obj = filter.as_object().unwrap_or(&empty);

        let mut where_clauses = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut order_clause = String::new();
        let mut limit_clause = String::new();
        // Captured raw, then bounded after the loop (see query_max_limit):
        // a client $limit is clamped to the cap and a missing one defaults
        // to it, so an uncapped SELECT can't materialize a whole table.
        let mut client_limit: Option<u64> = None;
        let mut client_offset: Option<u64> = None;
        let mut idx = 1;

        for (key, val) in obj {
            match key.as_str() {
                "$order" => {
                    if let Some(o) = val.as_object() {
                        let mut parts: Vec<String> = Vec::new();
                        for (col, dir) in o {
                            validate_column_name(col, ent)?;
                            let d = match dir.as_str().unwrap_or("asc") {
                                "desc" | "DESC" => "DESC",
                                _ => "ASC",
                            };
                            parts.push(format!("{} {d}", quote_ident(col)));
                        }
                        if !parts.is_empty() {
                            order_clause = format!(" ORDER BY {}", parts.join(", "));
                        }
                    }
                }
                "$limit" => {
                    if let Some(n) = val.as_u64() {
                        client_limit = Some(n);
                    }
                }
                "$offset" => {
                    if let Some(n) = val.as_u64() {
                        client_offset = Some(n);
                    }
                }
                _ => {
                    validate_column_name(key, ent)?;
                    let qk = quote_ident(key);
                    // Same json whole-value equality rule as the main
                    // query_filtered loop above.
                    if entity_field_is_json(ent, key)
                        && val
                            .as_object()
                            .is_some_and(|o| !o.keys().any(|k| k.starts_with('$')))
                    {
                        where_clauses.push(format!("{qk} = ?{idx}"));
                        values.push(json_to_sql_typed(ent, key, val));
                        idx += 1;
                    } else if let Some(op_obj) = val.as_object() {
                        for (op, op_val) in op_obj {
                            match op.as_str() {
                                "$not" => {
                                    where_clauses.push(format!("{qk} != ?{idx}"));
                                    values.push(json_to_sql_typed(ent, key, op_val));
                                    idx += 1;
                                }
                                "$gt" => {
                                    where_clauses.push(format!("{qk} > ?{idx}"));
                                    values.push(json_to_sql_typed(ent, key, op_val));
                                    idx += 1;
                                }
                                "$gte" => {
                                    where_clauses.push(format!("{qk} >= ?{idx}"));
                                    values.push(json_to_sql_typed(ent, key, op_val));
                                    idx += 1;
                                }
                                "$lt" => {
                                    where_clauses.push(format!("{qk} < ?{idx}"));
                                    values.push(json_to_sql_typed(ent, key, op_val));
                                    idx += 1;
                                }
                                "$lte" => {
                                    where_clauses.push(format!("{qk} <= ?{idx}"));
                                    values.push(json_to_sql_typed(ent, key, op_val));
                                    idx += 1;
                                }
                                "$like" => {
                                    where_clauses.push(format!("{qk} LIKE ?{idx}"));
                                    let p = format!("%{}%", op_val.as_str().unwrap_or(""));
                                    values.push(Box::new(p));
                                    idx += 1;
                                }
                                "$in" => {
                                    if let Some(arr) = op_val.as_array() {
                                        let ph: Vec<String> = arr
                                            .iter()
                                            .map(|v| {
                                                let p = format!("?{idx}");
                                                values.push(json_to_sql_typed(ent, key, v));
                                                idx += 1;
                                                p
                                            })
                                            .collect();
                                        if !ph.is_empty() {
                                            where_clauses
                                                .push(format!("{qk} IN ({})", ph.join(", ")));
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else {
                        where_clauses.push(format!("{qk} = ?{idx}"));
                        values.push(json_to_sql_typed(ent, key, val));
                        idx += 1;
                    }
                }
            }
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };
        if order_clause.is_empty() {
            order_clause = " ORDER BY \"id\"".into();
        }

        // Bound the result set: clamp a client $limit and default a missing
        // one to the cap, so `{}` can't stream the whole table into memory.
        let effective_limit = pylon_kernel::util::effective_query_limit(client_limit);
        limit_clause = match client_offset {
            Some(off) => format!(" LIMIT {effective_limit} OFFSET {off}"),
            None => format!(" LIMIT {effective_limit}"),
        };
        let sql = format!(
            "SELECT * FROM {}{}{}{}",
            quote_ident(entity),
            where_sql,
            order_clause,
            limit_clause
        );
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|v| v.as_ref()).collect();
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare: {e}"),
        })?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_json(row, &fields)))
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Query failed: {e}"),
            })?;
        Ok(rows.flatten().collect())
    }

    /// Graph query using a pre-held connection (for transactions).
    pub fn query_graph_with_conn(
        &self,
        conn: &Connection,
        query: &serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        let obj = query.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_QUERY".into(),
            message: "Graph query must be a JSON object".into(),
        })?;
        let mut results = serde_json::Map::new();
        for (entity_name, query_opts) in obj {
            let _ent = self.require_entity(entity_name)?;
            let filter = query_opts
                .get("where")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            let rows = self.query_filtered_with_conn(conn, entity_name, &filter)?;
            results.insert(entity_name.clone(), serde_json::json!(rows));
        }
        Ok(serde_json::Value::Object(results))
    }

    // -----------------------------------------------------------------------
    // Aggregations — count, sum, avg, min, max, group by
    // -----------------------------------------------------------------------

    /// Run an aggregation query. See [`pylon_http::DataStore::aggregate`]
    /// for the spec shape.
    pub fn aggregate(
        &self,
        entity: &str,
        spec: &serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        if let Some(pg) = self.pg_backend() {
            return pylon_http::DataStore::aggregate(&pg.store, entity, spec)
                .map_err(data_err_to_runtime);
        }
        let ent = self.require_entity(entity)?;
        let conn = self.lock_read_conn()?;
        let obj = spec.as_object().ok_or_else(|| RuntimeError {
            code: "INVALID_QUERY".into(),
            message: "aggregate spec must be an object".into(),
        })?;

        // Build the SELECT list.
        let mut select_parts: Vec<String> = Vec::new();
        let mut result_fields: Vec<String> = Vec::new();

        if let Some(count) = obj.get("count") {
            match count {
                serde_json::Value::String(s) if s == "*" => {
                    select_parts.push("COUNT(*) AS count".into());
                    result_fields.push("count".into());
                }
                serde_json::Value::String(field) => {
                    validate_column_name(field, ent)?;
                    let alias = format!("count_{field}");
                    select_parts.push(format!(
                        "COUNT({}) AS {}",
                        quote_ident(field),
                        quote_ident(&alias)
                    ));
                    result_fields.push(alias);
                }
                _ => {}
            }
        }

        for (fn_name, alias_prefix) in [
            ("sum", "sum_"),
            ("avg", "avg_"),
            ("min", "min_"),
            ("max", "max_"),
        ] {
            if let Some(fields) = obj.get(fn_name).and_then(|v| v.as_array()) {
                for field in fields {
                    if let Some(f) = field.as_str() {
                        validate_column_name(f, ent)?;
                        let alias = format!("{alias_prefix}{f}");
                        let sql_fn = fn_name.to_uppercase();
                        select_parts.push(format!(
                            "{}({}) AS {}",
                            sql_fn,
                            quote_ident(f),
                            quote_ident(&alias)
                        ));
                        result_fields.push(alias);
                    }
                }
            }
        }

        // countDistinct — separate handler because COUNT(DISTINCT) is a
        // distinct SQL form from COUNT(field). Lets dashboards ask "how
        // many unique customers placed orders this month" without a
        // client-side post-processing pass.
        if let Some(fields) = obj.get("countDistinct").and_then(|v| v.as_array()) {
            for field in fields {
                if let Some(f) = field.as_str() {
                    validate_column_name(f, ent)?;
                    let alias = format!("count_distinct_{f}");
                    select_parts.push(format!(
                        "COUNT(DISTINCT {}) AS {}",
                        quote_ident(f),
                        quote_ident(&alias)
                    ));
                    result_fields.push(alias);
                }
            }
        }

        // Group-by fields come first in the SELECT so each row is identifiable.
        // Each entry is either a plain column name (string) or a date-bucket
        // spec — `{ field: "createdAt", bucket: "day" }`. Buckets map to
        // SQLite strftime patterns so aggregation keys collapse to the
        // bucket boundary (hour / day / week / month / year).
        let mut group_by: Vec<String> = Vec::new();
        let mut group_select: Vec<String> = Vec::new();
        let mut group_field_names: Vec<String> = Vec::new();
        if let Some(groups) = obj.get("groupBy").and_then(|v| v.as_array()) {
            for g in groups {
                if let Some(f) = g.as_str() {
                    validate_column_name(f, ent)?;
                    let quoted = quote_ident(f);
                    group_by.push(quoted.clone());
                    group_select.push(quoted);
                    group_field_names.push(f.to_string());
                } else if let Some(spec) = g.as_object() {
                    let field =
                        spec.get("field")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| RuntimeError {
                                code: "INVALID_QUERY".into(),
                                message: "groupBy object spec requires `field`".into(),
                            })?;
                    validate_column_name(field, ent)?;
                    let bucket = spec.get("bucket").and_then(|v| v.as_str()).unwrap_or("day");
                    let fmt = match bucket {
                        "hour" => "%Y-%m-%d %H:00:00",
                        "day" => "%Y-%m-%d",
                        "month" => "%Y-%m",
                        "year" => "%Y",
                        "week" => "%Y-W%W",
                        _ => {
                            return Err(RuntimeError {
                                code: "INVALID_QUERY".into(),
                                message: format!(
                                    "bucket must be one of hour/day/week/month/year, got {bucket}"
                                ),
                            });
                        }
                    };
                    let alias = format!("{field}_{bucket}");
                    let expr = format!("strftime('{}', {})", fmt, quote_ident(field));
                    group_by.push(expr.clone());
                    group_select.push(format!("{} AS {}", expr, quote_ident(&alias)));
                    group_field_names.push(alias);
                }
            }
        }
        let mut full_select = group_select.clone();
        full_select.extend(select_parts.iter().cloned());
        if full_select.is_empty() {
            return Err(RuntimeError {
                code: "INVALID_QUERY".into(),
                message: "aggregate spec must include count/sum/avg/min/max/groupBy".into(),
            });
        }

        // WHERE clause (reuse filter syntax, but only simple equality for now).
        let mut where_clauses = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut idx = 1;
        if let Some(where_obj) = obj.get("where").and_then(|v| v.as_object()) {
            for (k, v) in where_obj {
                validate_column_name(k, ent)?;
                where_clauses.push(format!("{} = ?{idx}", quote_ident(k)));
                values.push(json_to_sql_typed(ent, k, v));
                idx += 1;
            }
        }
        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };

        let group_sql = if group_by.is_empty() {
            String::new()
        } else {
            format!(" GROUP BY {}", group_by.join(", "))
        };

        let sql = format!(
            "SELECT {} FROM {}{}{}",
            full_select.join(", "),
            quote_ident(entity),
            where_sql,
            group_sql
        );

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|v| v.as_ref()).collect();
        let mut stmt = conn.prepare_cached(&sql).map_err(|e| RuntimeError {
            code: "QUERY_FAILED".into(),
            message: format!("Failed to prepare aggregate: {e}"),
        })?;

        let column_names: Vec<String> = {
            let mut v = group_field_names.clone();
            v.extend(result_fields.iter().cloned());
            v
        };

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let mut obj = serde_json::Map::new();
                for (i, name) in column_names.iter().enumerate() {
                    // Try int first (counts/sums), then float, then string, then null.
                    if let Ok(n) = row.get::<_, i64>(i) {
                        obj.insert(name.clone(), serde_json::Value::Number(n.into()));
                    } else if let Ok(f) = row.get::<_, f64>(i) {
                        if let Some(num) = serde_json::Number::from_f64(f) {
                            obj.insert(name.clone(), serde_json::Value::Number(num));
                        } else {
                            obj.insert(name.clone(), serde_json::Value::Null);
                        }
                    } else if let Ok(s) = row.get::<_, String>(i) {
                        obj.insert(name.clone(), serde_json::Value::String(s));
                    } else {
                        obj.insert(name.clone(), serde_json::Value::Null);
                    }
                }
                Ok(serde_json::Value::Object(obj))
            })
            .map_err(|e| RuntimeError {
                code: "QUERY_FAILED".into(),
                message: format!("Aggregate failed: {e}"),
            })?;

        let mut result = Vec::new();
        for row in rows {
            if let Ok(val) = row {
                result.push(val);
            }
        }
        Ok(serde_json::json!({ "rows": result }))
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn require_entity(&self, name: &str) -> Result<&ManifestEntity, RuntimeError> {
        self.entities.get(name).ok_or_else(|| RuntimeError {
            code: "ENTITY_NOT_FOUND".into(),
            message: format!("Unknown entity: \"{name}\""),
        })
    }

    /// Acquire the write connection. Used for INSERT, UPDATE, DELETE.
    /// SQLite-only — Postgres callers should never reach this (each
    /// public CRUD method branches at the top and dispatches to
    /// `PostgresDataStore` first). Returns `NOT_SQLITE_BACKEND` if
    /// invoked on a Postgres runtime, which indicates a missing dispatch.
    fn lock_write_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, RuntimeError> {
        let sb = self.sqlite_backend()?;
        sb.write_conn.lock().map_err(|e| RuntimeError {
            code: "LOCK_FAILED".into(),
            message: format!("Failed to acquire write connection lock: {e}"),
        })
    }

    /// Acquire a read connection. Uses the read pool if available (file-backed
    /// databases), otherwise falls back to the write connection (in-memory).
    /// Connections are selected round-robin to spread load evenly. SQLite-only.
    fn lock_read_conn(&self) -> Result<ReadConnGuard<'_>, RuntimeError> {
        let sb = self.sqlite_backend()?;
        if !sb.read_pool.is_empty() {
            let idx = sb.read_counter.fetch_add(1, Ordering::Relaxed) % sb.read_pool.len();
            let guard = sb.read_pool[idx].lock().map_err(|e| RuntimeError {
                code: "LOCK_FAILED".into(),
                message: format!("Failed to acquire read connection: {e}"),
            })?;
            Ok(ReadConnGuard::Pooled(guard))
        } else {
            // Fall back to write connection for in-memory DBs.
            let guard = sb.write_conn.lock().map_err(|e| RuntimeError {
                code: "LOCK_FAILED".into(),
                message: format!("Failed to acquire connection: {e}"),
            })?;
            Ok(ReadConnGuard::Write(guard))
        }
    }

    /// Force-flush + truncate the WAL on the main write connection. Called
    /// from the server's shutdown drain so the next boot doesn't have to
    /// recover a stale WAL. Postgres-backed runtimes are a no-op (the
    /// only auxiliary SQLite dbs Pylon owns are session/jobs/etc. — those
    /// have their own shutdown via SessionStore/JobQueue or just rely on
    /// auto-checkpoint).
    ///
    /// Errors are logged by the caller but not fatal — checkpointing is
    /// hygiene, not correctness; a missed checkpoint just means the next
    /// boot does the recovery itself (which now has busy_timeout=5s so
    /// it can't hang).
    pub fn checkpoint_wal(&self) -> Result<(), RuntimeError> {
        let sb = match self.sqlite_backend() {
            Ok(sb) => sb,
            Err(_) => return Ok(()), // Postgres or not-sqlite — nothing to do
        };
        let conn = sb.write_conn.lock().map_err(|e| RuntimeError {
            code: "LOCK_FAILED".into(),
            message: format!("Failed to acquire write conn for checkpoint: {e}"),
        })?;
        // wal_checkpoint(TRUNCATE) blocks until the WAL is fully merged
        // into the main DB AND truncated to zero bytes. Safe under
        // concurrent readers (they'll just retry); we drain HTTP
        // workers first so the only contender is the scheduler/job
        // worker tail which should also have stopped by this point.
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")
            .map_err(|e| RuntimeError {
                code: "WAL_CHECKPOINT_FAILED".into(),
                message: format!("wal_checkpoint(TRUNCATE) failed: {e}"),
            })
    }

    /// Borrow the SQLite backend, or fail with `NOT_SQLITE_BACKEND` if
    /// this runtime is Postgres-backed. Used by every SQLite-specific
    /// helper as a single point of dispatch.
    fn sqlite_backend(&self) -> Result<&SqliteBackend, RuntimeError> {
        match &self.backend {
            RuntimeBackend::Sqlite(sb) => Ok(sb),
            RuntimeBackend::Postgres(_) => Err(RuntimeError {
                code: "NOT_SQLITE_BACKEND".into(),
                message: "this operation requires a SQLite-backed Runtime".into(),
            }),
        }
    }

    /// Borrow the Postgres backend, or `None` for SQLite. Used by the
    /// per-method dispatch at the top of each entity-CRUD function
    /// AND by the `DataStore` impl in `datastore.rs` to reach the
    /// CRDT sidecar.
    pub(crate) fn pg_backend(&self) -> Option<&PgBackend> {
        match &self.backend {
            RuntimeBackend::Sqlite(_) => None,
            RuntimeBackend::Postgres(pg) => Some(pg),
        }
    }

    /// Borrow the underlying Postgres `DataStore` if this runtime is
    /// Postgres-backed. Used by the `DataStore` adapter in `datastore.rs`
    /// to delegate `transact`/`search` etc. without re-implementing them.
    /// Accessor for the underlying PostgresDataStore. Used by
    /// integration tests to exercise in-tx primitives directly
    /// without going through a TS function handler. Also useful for
    /// callers that need to drop down to raw PG (e.g. running an
    /// EXPLAIN against the live cluster from an admin tool).
    /// Returns None on SQLite-backed runtimes.
    pub fn pg_data_store_pub(&self) -> Option<&pylon_storage::pg_datastore::PostgresDataStore> {
        self.pg_data_store()
    }

    #[doc(hidden)]
    pub fn pg_data_store_for_tests(&self) -> &pylon_storage::pg_datastore::PostgresDataStore {
        self.pg_data_store().expect("pg backend")
    }

    /// Test-only: run a closure inside a PG mutation tx with the
    /// CRDT hook installed — same code path FnOpsImpl::call uses
    /// for `Mutation` handlers. Lets integration tests verify the
    /// hook without spinning up a Bun runtime.
    #[doc(hidden)]
    pub fn run_in_pg_mutation_tx_for_tests<F, T, E>(&self, body: F) -> Result<T, E>
    where
        F: FnOnce(&dyn pylon_http::DataStore) -> Result<T, E>,
        E: From<pylon_http::DataError>,
    {
        let pg_backend = self.pg_backend().expect("pg backend");
        let crdt_hook: std::sync::Arc<dyn pylon_storage::pg_tx_store::PgCrdtHook> =
            std::sync::Arc::new(crate::pg_loro_store::PgCrdtHookImpl {
                crdt: std::sync::Arc::clone(&pg_backend.crdt),
                manifest: Arc::clone(&self.manifest),
            });
        pg_backend.store.with_transaction_crdt(crdt_hook, body)
    }

    pub(crate) fn pg_data_store(&self) -> Option<&pylon_storage::pg_datastore::PostgresDataStore> {
        self.pg_backend().map(|pg| &pg.store)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generate a lex-sortable, monotonic-ish unique ID.
///
/// Same shape as `pylon_storage::postgres::generate_id` — fixed-width hex
/// of nanoseconds + 8-hex per-process counter (40 chars total). The fixed
/// width is what makes `WHERE id > $1 ORDER BY id` correct for cursor
/// pagination: variable-width hex sorts incorrectly at width boundaries
/// (e.g. `"ff"` lex-sorts after `"100"`).
/// Run `body` inside a SQLite transaction on `conn`. Commits on `Ok`,
/// rolls back on `Err` (or if `body` panics).
///
/// Used to make the multi-statement CRDT write paths (LoroDoc snapshot
/// upsert into `_pylon_crdt_snapshots` + the materialized entity row
/// INSERT/UPDATE + FTS / facet maintenance) atomic so a crash mid-write
/// can never leave the materialized view stale relative to the CRDT
/// snapshot. Uses unmanaged BEGIN/COMMIT/ROLLBACK rather than rusqlite's
/// `Transaction` API because the existing call sites borrow `conn`
/// through inner closures and the lifetime juggling for a `Transaction`
/// guard would force more refactoring than the explicit BEGIN/COMMIT.
///
/// `BEGIN IMMEDIATE` (vs the default `BEGIN DEFERRED`) takes the SQLite
/// reserved lock on entry instead of escalating later — matches the
/// pattern in `datastore.rs::transact` and avoids a SQLITE_BUSY race
/// where a concurrent reader prevents the lock upgrade mid-write.
fn with_write_tx<T, F>(conn: &rusqlite::Connection, body: F) -> Result<T, RuntimeError>
where
    F: FnOnce() -> Result<T, RuntimeError>,
{
    conn.execute("BEGIN IMMEDIATE", [])
        .map_err(|e| RuntimeError {
            code: "TX_BEGIN_FAILED".into(),
            message: format!("BEGIN: {e}"),
        })?;
    match body() {
        Ok(v) => {
            conn.execute("COMMIT", []).map_err(|e| RuntimeError {
                code: "TX_COMMIT_FAILED".into(),
                message: format!("COMMIT: {e}"),
            })?;
            Ok(v)
        }
        Err(e) => {
            // Best-effort rollback; if even ROLLBACK fails we surface
            // the *original* error since that's the more actionable one.
            let _ = conn.execute("ROLLBACK", []);
            Err(e)
        }
    }
}

fn generate_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:032x}{seq:08x}")
}

/// Honor a caller-supplied `id` if it's well-formed, otherwise generate
/// one. This is the entry point for client-provided ids on optimistic
/// mutations: the React `useMutation({ optimistic })` hook generates a
/// Pylon-shaped id with `@pylonsync/sync`'s `generateId()`, threads it
/// through the mutation args, and the server function passes it on to
/// `ctx.db.insert("Entity", { id, ... })`. By accepting the id here the
/// optimistic ghost the client painted and the canonical row the WS
/// broadcast carries share the same `row_id`, so the local store's
/// merge is a no-op instead of a delete-then-replace flash.
///
/// Format check is conservative — exactly 40 lowercase hex chars, the
/// same shape `generate_id` produces. This rejects ULIDs, UUIDs, slugs,
/// and "user_42" style ids that would silently break cursor pagination
/// (which assumes lex-sortable fixed-width ids; see `generate_id`'s doc
/// comment for why).
/// Cheap pre-check: does this entity declare any field with a
/// `default` on it? If not, skip the clone in `insert()`. Most
/// app entities don't use defaults, so this keeps the hot path
/// allocation-free.
pub(crate) fn entity_has_any_default(manifest: &pylon_kernel::AppManifest, entity: &str) -> bool {
    manifest
        .entities
        .iter()
        .find(|e| e.name == entity)
        .map(|e| e.fields.iter().any(|f| f.default.is_some()))
        .unwrap_or(false)
}

/// Walk the entity's manifest fields; for each one with a `default`
/// declared, fill it in on the row if absent. Existing values in the
/// row always win — only TRULY absent or `null` slots get defaulted.
///
/// `"now"` is the special marker for `field.datetime().defaultNow()`;
/// any other literal is stamped as-is.
pub(crate) fn apply_field_defaults(
    manifest: &pylon_kernel::AppManifest,
    entity: &str,
    data: &serde_json::Value,
) -> serde_json::Value {
    let ent = match manifest.entities.iter().find(|e| e.name == entity) {
        Some(e) => e,
        None => return data.clone(),
    };
    let mut out = data.clone();
    let obj = match out.as_object_mut() {
        Some(o) => o,
        // Non-object payload (e.g. transact ops that already produced
        // a normalized row). Don't try to mutate — let the backend
        // handle the shape mismatch.
        None => return out,
    };
    for f in &ent.fields {
        let already_set = matches!(
            obj.get(&f.name),
            Some(v) if !v.is_null()
        );
        if already_set {
            continue;
        }
        let Some(default) = &f.default else { continue };
        // Dynamic, auth-derived defaults (e.g. `field.X().owner()` →
        // `{"$auth":"userId"}`) are filled by the auth-aware mutation
        // pipeline (OwnerStampPlugin), not here — this storage-layer
        // pass has no AuthContext, so stamping the literal sentinel
        // object as the value would both corrupt the row and defeat
        // the security guarantee. Skip them.
        if is_dynamic_default(default) {
            continue;
        }
        let value = if default == &serde_json::Value::String("now".to_string()) {
            serde_json::Value::String(crate::tinybird_logger::iso_now_ms())
        } else {
            default.clone()
        };
        obj.insert(f.name.clone(), value);
    }
    out
}

/// A manifest `default` is *dynamic* when its value can't be known
/// without request context — currently only the auth-derived owner
/// sentinel `{"$auth": "userId"}` emitted by `field.X().owner()`.
///
/// These are filled by the auth-aware plugin chain
/// ([`pylon_plugin::builtin::owner_stamp::OwnerStampPlugin`]), never by
/// the storage-layer `apply_field_defaults` pass. Keeping the predicate
/// here (next to the only consumer) means a new dynamic-default kind is
/// a one-line addition that automatically gets skipped at the storage
/// layer.
pub(crate) fn is_dynamic_default(default: &serde_json::Value) -> bool {
    default.as_object().and_then(|o| o.get("$auth")).is_some()
}

fn resolve_or_generate_id(data: &serde_json::Value) -> Result<String, RuntimeError> {
    let obj = match data.as_object() {
        Some(o) => o,
        None => return Ok(generate_id()),
    };
    match obj.get("id") {
        None | Some(serde_json::Value::Null) => Ok(generate_id()),
        Some(serde_json::Value::String(s)) => {
            if !is_valid_pylon_id(s) {
                return Err(RuntimeError {
                    code: "INVALID_ID".into(),
                    message: format!(
                        "client-provided `id` must be a 40-char lowercase hex string \
                         (use generateId() from @pylonsync/sync). Got: {s:?}"
                    ),
                });
            }
            Ok(s.clone())
        }
        Some(other) => Err(RuntimeError {
            code: "INVALID_ID".into(),
            message: format!(
                "Insert data carried a non-string `id` value: {other}. \
                 Pylon row ids are always 40-char hex strings."
            ),
        }),
    }
}

fn is_valid_pylon_id(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Field-aware variant of [`json_to_sql`]: a `json`-typed column ALWAYS
/// binds the serialized JSON TEXT of the value — `{"a":1}` binds as
/// `{"a":1}`, but so does the bare string `"42"` bind as `"\"42\""` —
/// so every JSON value round-trips exactly through the TEXT column and
/// `parse_json_fields_on_read`. Every other field keeps the
/// shape-driven binding. Applied at the SQL boundary only: the
/// in-memory row (change events, hooks, policies) always carries the
/// parsed value.
fn json_to_sql_typed(
    ent: &pylon_kernel::ManifestEntity,
    key: &str,
    val: &serde_json::Value,
) -> Box<dyn rusqlite::types::ToSql> {
    if entity_field_is_json(ent, key) {
        return Box::new(serde_json::to_string(val).unwrap_or_else(|_| "null".to_string()));
    }
    if let Some(dims) = entity_field_vector_dims(ent, key) {
        // `validate_vector_fields` already rejected malformed writes at
        // the mutation entry points; anything unpackable here (e.g. a
        // filter value of the wrong shape) binds NULL, which matches
        // nothing.
        return match pylon_storage::vector::json_to_f32s(val, dims) {
            Ok(f) => Box::new(pylon_storage::vector::pack_f32(&f)),
            Err(_) => Box::new(rusqlite::types::Null),
        };
    }
    json_to_sql(val)
}

/// Parse each `json`-typed field's stored TEXT back to the real value.
/// Idempotent: only `Value::String` cells are touched, so a row that
/// already carries parsed values (e.g. a Postgres JSONB read, or a
/// double-normalized path) passes through unchanged. Text that doesn't
/// parse (hand-written legacy rows) stays a string rather than erroring
/// the whole read.
pub(crate) fn parse_json_fields_in_row(
    ent: &pylon_kernel::ManifestEntity,
    row: &mut serde_json::Value,
) {
    if !ent.fields.iter().any(|f| f.field_type == "json") {
        return;
    }
    let Some(obj) = row.as_object_mut() else {
        return;
    };
    for f in &ent.fields {
        if f.field_type != "json" {
            continue;
        }
        let Some(serde_json::Value::String(s)) = obj.get(&f.name) else {
            continue;
        };
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s) {
            obj.insert(f.name.clone(), parsed);
        }
    }
}

fn entity_field_is_json(ent: &pylon_kernel::ManifestEntity, key: &str) -> bool {
    ent.fields
        .iter()
        .any(|f| f.name == key && f.field_type == "json")
}

/// Declared dims when `key` is a `vector(dims)` field, else `None`.
pub(crate) fn entity_field_vector_dims(
    ent: &pylon_kernel::ManifestEntity,
    key: &str,
) -> Option<u32> {
    ent.fields
        .iter()
        .find(|f| f.name == key)
        .and_then(|f| pylon_storage::vector::vector_dims(&f.field_type))
}

/// Reject writes whose vector-field values are not finite number
/// arrays of the declared dimension. Called at every mutation entry
/// point (insert / update, direct and in-transaction) so a bad
/// embedding fails loudly with a typed error instead of landing as an
/// unsearchable NULL blob. JSON `null` passes — that's how an optional
/// embedding is cleared.
pub(crate) fn validate_vector_fields(
    ent: &pylon_kernel::ManifestEntity,
    data: &serde_json::Value,
) -> Result<(), RuntimeError> {
    let Some(obj) = data.as_object() else {
        return Ok(());
    };
    for f in &ent.fields {
        let Some(dims) = pylon_storage::vector::vector_dims(&f.field_type) else {
            continue;
        };
        let Some(v) = obj.get(&f.name) else {
            continue;
        };
        if v.is_null() {
            continue;
        }
        if let Err(e) = pylon_storage::vector::json_to_f32s(v, dims) {
            return Err(RuntimeError {
                code: "VECTOR_INVALID".into(),
                message: format!("{}.{}: {}", ent.name, f.name, e),
            });
        }
    }
    Ok(())
}

/// Parse vector fields on a row read back from storage. SQLite rows
/// already carry decoded arrays (see `row_to_json`); Postgres BYTEA
/// columns surface as base64 strings, which this converts to number
/// arrays. Idempotent — non-string values pass through.
pub(crate) fn parse_vector_fields_in_row(
    ent: &pylon_kernel::ManifestEntity,
    row: &mut serde_json::Value,
) {
    use base64::Engine as _;
    if !ent
        .fields
        .iter()
        .any(|f| f.field_type.starts_with("vector("))
    {
        return;
    }
    let Some(obj) = row.as_object_mut() else {
        return;
    };
    for f in &ent.fields {
        if pylon_storage::vector::vector_dims(&f.field_type).is_none() {
            continue;
        }
        let Some(serde_json::Value::String(s)) = obj.get(&f.name) else {
            continue;
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(s) else {
            continue;
        };
        if let Some(values) = pylon_storage::vector::unpack_f32(&bytes) {
            obj.insert(f.name.clone(), pylon_storage::vector::f32s_to_json(&values));
        }
    }
}

/// Postgres counterpart of [`json_to_sql_typed`]: the PG SQL builders
/// live in `pylon_storage` and never see the manifest, so `json` field
/// values are pre-serialized to `Value::String` in a CLONE of the row
/// right before the storage call. Returns `None` when the entity has no
/// json fields present in `data` (the common case — no clone).
pub(crate) fn serialize_json_fields_for_storage(
    ent: &pylon_kernel::ManifestEntity,
    data: &serde_json::Value,
) -> Option<serde_json::Value> {
    let obj = data.as_object()?;
    let needs_ser = |f: &pylon_kernel::ManifestField| {
        f.field_type == "json" || f.field_type.starts_with("vector(")
    };
    if !ent
        .fields
        .iter()
        .any(|f| needs_ser(f) && obj.contains_key(&f.name))
    {
        return None;
    }
    let mut out = obj.clone();
    for f in &ent.fields {
        if !needs_ser(f) {
            continue;
        }
        if let Some(v) = out.get(&f.name) {
            // Vector fields ride the same pre-serialize channel: the
            // JSON text of the number array. JsonParam's BYTEA arm
            // parses it back and packs LE f32 bytes at bind time.
            let ser = serde_json::to_string(v).unwrap_or_else(|_| "null".to_string());
            out.insert(f.name.clone(), serde_json::Value::String(ser));
        }
    }
    Some(serde_json::Value::Object(out))
}

/// Convert a `serde_json::Value` to a boxed `ToSql` for rusqlite.
fn json_to_sql(val: &serde_json::Value) -> Box<dyn rusqlite::types::ToSql> {
    match val {
        serde_json::Value::Null => Box::new(rusqlite::types::Null),
        serde_json::Value::Bool(b) => Box::new(*b as i32),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Box::new(i)
            } else if let Some(f) = n.as_f64() {
                Box::new(f)
            } else {
                Box::new(n.to_string())
            }
        }
        serde_json::Value::String(s) => Box::new(s.clone()),
        other => Box::new(other.to_string()),
    }
}

/// Convert a rusqlite row to a JSON value.
///
/// Reads columns by NAME (via the row's actual column metadata) rather
/// than by positional index. The previous implementation assumed the
/// SQLite table column order matched the manifest field order, which
/// silently breaks when a new field is inserted in the middle of the
/// manifest: SQLite's `ALTER TABLE ADD COLUMN` always appends to the
/// end of the table, so existing data lands in the wrong field on
/// every read.
///
/// `field_names` is still passed (unused in the body, kept for API
/// stability with callers that compute it from the manifest) — the
/// name set comes from the row itself now, which always matches the
/// SELECT's actual column shape.
fn row_to_json(row: &rusqlite::Row<'_>, fields: &[ManifestField]) -> serde_json::Value {
    let mut obj = serde_json::Map::new();

    let stmt = row.as_ref();
    let count = stmt.column_count();
    for i in 0..count {
        // Column names are short string slices into the prepared
        // statement; copy out into owned Strings before inserting into
        // the map (the slice borrow can't outlive the row).
        let name = match stmt.column_name(i) {
            Ok(n) => n.to_string(),
            Err(_) => continue,
        };
        let is_bool = fields
            .iter()
            .any(|f| f.name == name && f.field_type == "bool");
        let value = if let Ok(s) = row.get::<_, String>(i) {
            serde_json::Value::String(s)
        } else if let Ok(n) = row.get::<_, i64>(i) {
            // SQLite stores bool columns as INTEGER 0/1. Serving that raw
            // breaks every typed client (Swift's `0 → Bool` decode throws,
            // strict JS `=== false` checks fail) — map back to real JSON
            // booleans per the schema. Postgres is unaffected (native
            // BOOLEAN decodes to Value::Bool already).
            if is_bool {
                serde_json::Value::Bool(n != 0)
            } else {
                serde_json::Value::Number(serde_json::Number::from(n))
            }
        } else if let Ok(f) = row.get::<_, f64>(i) {
            serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null)
        } else if let Ok(b) = row.get::<_, Vec<u8>>(i) {
            // BLOB columns only come from `vector(dims)` fields — decode
            // the packed LE f32 array back to a number array. (TEXT and
            // numeric columns were caught by the branches above, so a
            // Vec<u8> read here really is a blob.)
            let is_vector = fields
                .iter()
                .any(|f| f.name == name && f.field_type.starts_with("vector("));
            if is_vector {
                pylon_storage::vector::unpack_f32(&b)
                    .map(|v| pylon_storage::vector::f32s_to_json(&v))
                    .unwrap_or(serde_json::Value::Null)
            } else {
                serde_json::Value::Null
            }
        } else {
            serde_json::Value::Null
        };
        obj.insert(name, value);
    }

    serde_json::Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::{ManifestField, ManifestIndex};

    fn test_manifest() -> AppManifest {
        AppManifest {
            manifest_version: 1,
            name: "Test".into(),
            version: "0.1.0".into(),
            entities: vec![pylon_kernel::ManifestEntity {
                name: "User".into(),
                fields: vec![
                    ManifestField {
                        name: "email".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: true,
                        crdt: None,
                        server_only: false,
                        readonly: false,
                        default: None,
                        enum_values: None,
                        encrypted: false,
                        sync_omit: false,
                    },
                    ManifestField {
                        name: "displayName".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: false,
                        crdt: None,
                        server_only: false,
                        readonly: false,
                        default: None,
                        enum_values: None,
                        encrypted: false,
                        sync_omit: false,
                    },
                ],
                indexes: vec![ManifestIndex {
                    name: "user_email".into(),
                    fields: vec!["email".into()],
                    unique: true,
                    where_clause: None,
                }],
                relations: vec![],
                search: None,
                crdt: true,
                sync: true,
                ..Default::default()
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        }
    }

    /// A manifest field built with `field.X().owner()` serializes its
    /// default as `{"$auth":"userId"}`. The storage-layer
    /// `apply_field_defaults` pass must NOT stamp that sentinel object
    /// as the field value — it has no auth context, and writing the
    /// literal `{"$auth":"userId"}` into the row would both corrupt the
    /// data and silently defeat the owner-stamp security guarantee
    /// (the row would carry a fake "owner" that's actually a sentinel,
    /// not a user id). Static literal defaults and `"now"` still fill.
    #[test]
    fn apply_field_defaults_skips_the_owner_auth_sentinel() {
        let owner_sentinel = serde_json::json!({ "$auth": "userId" });
        assert!(
            is_dynamic_default(&owner_sentinel),
            "the owner sentinel must be recognized as a dynamic default"
        );
        assert!(
            !is_dynamic_default(&serde_json::json!("now")),
            "\"now\" is a static default, not dynamic"
        );
        assert!(
            !is_dynamic_default(&serde_json::json!("active")),
            "a literal string default is not dynamic"
        );

        let mut m = test_manifest();
        let ent = &mut m.entities[0];
        ent.fields.push(ManifestField {
            name: "ownerId".into(),
            field_type: "string".into(),
            optional: false,
            unique: false,
            crdt: None,
            server_only: false,
            readonly: true,
            default: Some(owner_sentinel),
            enum_values: None,
            encrypted: false,
            sync_omit: false,
        });
        ent.fields.push(ManifestField {
            name: "status".into(),
            field_type: "string".into(),
            optional: false,
            unique: false,
            crdt: None,
            server_only: false,
            readonly: false,
            default: Some(serde_json::json!("active")),
            enum_values: None,
            encrypted: false,
            sync_omit: false,
        });

        let row = serde_json::json!({ "email": "a@b.com", "displayName": "A" });
        let filled = apply_field_defaults(&m, "User", &row);

        // The literal default fills as usual...
        assert_eq!(filled["status"], serde_json::json!("active"));
        // ...but the owner sentinel is left absent for the auth-aware
        // OwnerStampPlugin to fill — it must NOT be stamped as a value.
        assert!(
            filled.get("ownerId").is_none(),
            "owner sentinel must not be materialized at the storage layer; got {:?}",
            filled.get("ownerId")
        );
    }

    #[test]
    fn reset_for_tests_wipes_in_memory() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        rt.insert(
            "User",
            &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
        )
        .unwrap();
        assert_eq!(rt.list("User").unwrap().len(), 1);
        rt.reset_for_tests().unwrap();
        assert_eq!(rt.list("User").unwrap().len(), 0);
    }

    #[test]
    fn reset_for_tests_refuses_file_db() {
        let dir = std::env::temp_dir().join("pylon-reset-refuse");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("db.sqlite");
        let _ = std::fs::remove_file(&db_path);
        let rt = Runtime::open(db_path.to_str().unwrap(), test_manifest()).unwrap();
        let err = rt.reset_for_tests().unwrap_err();
        assert_eq!(err.code, "RESET_REFUSED");
        let _ = std::fs::remove_file(&db_path);
    }

    /// Regression: a SQLite-backed runtime must hand back strictly
    /// monotonic seqs across process restarts. Without the persisted
    /// `_pylon_change_seq` row, every restart starts the in-memory
    /// counter at 0 and seed events fill 1..N; any client cursor from
    /// the prior incarnation that's >= N permanently 410s — observed
    /// in production as "cursor last_seq=49 is older than the oldest
    /// retained seq=49" after a deploy roll.
    ///
    /// Pinned at the runtime-helper layer; SqliteSeqAllocator's own
    /// monotonic-across-restart test (in crate::seq_allocator)
    /// exercises the full allocator wrapper.
    #[test]
    fn sqlite_change_seq_persists_across_restart() {
        let dir = std::env::temp_dir().join("pylon-seq-persist");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("db.sqlite");
        let _ = std::fs::remove_file(&db_path);

        {
            let rt = Runtime::open(db_path.to_str().unwrap(), test_manifest()).unwrap();
            rt.bootstrap_sqlite_change_seq().unwrap();
            assert_eq!(rt.current_sqlite_change_seq().unwrap(), 0);
            // Reserve 100 seqs. After: persisted = 100, returned = 100.
            assert_eq!(rt.reserve_sqlite_change_seq(100).unwrap(), 100);
            // Reserve another 50. After: persisted = 150.
            assert_eq!(rt.reserve_sqlite_change_seq(50).unwrap(), 150);
            assert_eq!(rt.current_sqlite_change_seq().unwrap(), 150);
        }
        {
            // Reopen the same file — simulating a process restart. The
            // counter must resume from 150, not 0.
            let rt = Runtime::open(db_path.to_str().unwrap(), test_manifest()).unwrap();
            rt.bootstrap_sqlite_change_seq().unwrap();
            assert_eq!(
                rt.current_sqlite_change_seq().unwrap(),
                150,
                "restart must resume from persisted seq, not reset to 0"
            );
            // Next reservation continues from there.
            assert_eq!(rt.reserve_sqlite_change_seq(25).unwrap(), 175);
        }

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn insert_and_get() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
            )
            .unwrap();
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["email"], "a@b.com");
    }

    /// Optimistic-mutation contract: a well-formed client-provided id
    /// is used verbatim for the inserted row, so the optimistic ghost
    /// the client painted and the canonical broadcast share the same
    /// `row_id` and the WS update is an in-place merge.
    #[test]
    fn insert_honors_client_provided_id() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let client_id = "0123456789abcdef0123456789abcdef01234567";
        let id = rt
            .insert(
                "User",
                &serde_json::json!({
                    "id": client_id,
                    "email": "client@example.com",
                    "displayName": "C",
                }),
            )
            .unwrap();
        assert_eq!(id, client_id, "runtime must echo client-provided id");
        let row = rt.get_by_id("User", client_id).unwrap().unwrap();
        assert_eq!(row["email"], "client@example.com");
    }

    /// A client-provided id that doesn't match Pylon's 40-char hex
    /// shape gets rejected before it touches the database. Without
    /// this guard, ULIDs and slugs would corrupt cursor pagination
    /// (which assumes lex-sortable fixed-width ids).
    #[test]
    fn insert_rejects_malformed_client_id() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let err = rt
            .insert(
                "User",
                &serde_json::json!({
                    "id": "01HX9YPK0V3J6Y0K7X2KZ8RQGT", // ULID — 26 chars
                    "email": "ulid@example.com",
                    "displayName": "U",
                }),
            )
            .expect_err("ULID must be rejected");
        assert_eq!(err.code, "INVALID_ID");
    }

    /// PK collision on a client-provided id surfaces as a typed
    /// OPTIMISTIC_ID_CONFLICT — distinct from a generic INSERT_FAILED
    /// — so retry logic on the client can mint a fresh id and re-issue
    /// instead of treating the collision as a fatal write error.
    #[test]
    fn insert_id_collision_returns_typed_error() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let client_id = "fedcba9876543210fedcba9876543210fedcba98";
        rt.insert(
            "User",
            &serde_json::json!({
                "id": client_id,
                "email": "first@example.com",
                "displayName": "First",
            }),
        )
        .unwrap();
        let err = rt
            .insert(
                "User",
                &serde_json::json!({
                    "id": client_id,
                    "email": "second@example.com",
                    "displayName": "Second",
                }),
            )
            .expect_err("duplicate id must fail");
        assert_eq!(err.code, "OPTIMISTIC_ID_CONFLICT");
    }

    /// Regression: when a new field is added in the middle of a manifest,
    /// SQLite ALTER TABLE ADD COLUMN appends it to the end of the table.
    /// The previous `row_to_json` read columns by positional index in
    /// manifest order, so existing data shifted into the wrong fields
    /// on every read (createdAt's value showed up as the new field's,
    /// and vice versa). row_to_json now reads by column name from the
    /// row's own metadata, so the bug can't recur regardless of
    /// migration order.
    #[test]
    fn row_to_json_handles_columns_added_out_of_manifest_order() {
        // Manifest: id, email, displayName, avatarColor, createdAt
        let mut manifest = test_manifest();
        manifest.entities[0].fields = vec![
            ManifestField {
                name: "email".into(),
                field_type: "string".into(),
                optional: false,
                unique: true,
                crdt: None,
                server_only: false,
                readonly: false,
                default: None,
                enum_values: None,
                encrypted: false,
                sync_omit: false,
            },
            ManifestField {
                name: "displayName".into(),
                field_type: "string".into(),
                optional: false,
                unique: false,
                crdt: None,
                server_only: false,
                readonly: false,
                default: None,
                enum_values: None,
                encrypted: false,
                sync_omit: false,
            },
            ManifestField {
                name: "avatarColor".into(),
                field_type: "string".into(),
                optional: true,
                unique: false,
                crdt: None,
                server_only: false,
                readonly: false,
                default: None,
                enum_values: None,
                encrypted: false,
                sync_omit: false,
            },
            ManifestField {
                name: "createdAt".into(),
                field_type: "datetime".into(),
                optional: true,
                unique: false,
                crdt: None,
                server_only: false,
                readonly: false,
                default: None,
                enum_values: None,
                encrypted: false,
                sync_omit: false,
            },
        ];
        // Important: turn off CRDT mode for this test — CRDT mode writes
        // the projection back to SQLite explicitly per-field, so it
        // wouldn't exercise the column-order bug we're regressing
        // against. The bug bites the legacy path that still does
        // `INSERT (id, email, displayName, ...) VALUES (...)` and then
        // `SELECT * ... → row_to_json` to read it back.
        manifest.entities[0].crdt = false;
        let rt = Runtime::in_memory(manifest).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({
                    "email": "a@b.com",
                    "displayName": "Alice",
                    "avatarColor": "#abc",
                    "createdAt": "2026-01-01T00:00:00Z",
                }),
            )
            .unwrap();

        // Simulate an ALTER TABLE ADD COLUMN that appends a new field
        // at the end of the SQLite table even though the manifest
        // places it in the middle. This is the exact shape of what
        // happens when a user adds a new field between existing ones
        // and pylon dev migrates the table forward.
        {
            let conn = rt.lock_write_conn().unwrap();
            conn.execute("ALTER TABLE \"User\" ADD COLUMN \"passwordHash\" TEXT", [])
                .unwrap();
            conn.execute(
                "UPDATE \"User\" SET \"passwordHash\" = ?1 WHERE \"id\" = ?2",
                rusqlite::params!["hashed-password", &id],
            )
            .unwrap();
        }
        // Update the in-memory manifest to reflect the new field
        // sitting between avatarColor and createdAt — this is what the
        // regenerated manifest would look like.
        // (We mutate via the storage path to mirror the actual flow.)

        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        // The crucial assertions: each column maps to its own value,
        // not the value of whichever column happens to share its
        // SQLite position.
        assert_eq!(row["email"], "a@b.com");
        assert_eq!(row["displayName"], "Alice");
        assert_eq!(row["avatarColor"], "#abc");
        assert_eq!(row["createdAt"], "2026-01-01T00:00:00Z");
        assert_eq!(row["passwordHash"], "hashed-password");
    }

    /// Regression: when an entity's text-field set changes, its
    /// `<Entity>_fts` FTS5 table must be rebuilt at boot. Migrations only
    /// ALTER the entity table; before this fix the FTS table (created
    /// IF NOT EXISTS) kept its old columns while the sync triggers —
    /// dropped alongside a SQLite table-rebuild migration and recreated
    /// at boot from the CURRENT manifest — referenced the new field. The
    /// first INSERT then failed with "table <Entity>_fts has no column
    /// named <newField>" (hit live: Batch gained `backgrounds`).
    #[test]
    fn fts_table_rebuilt_when_entity_gains_a_text_field() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("app.db");
        let db = db_path.to_str().unwrap();

        // v1: User(email, displayName). Write one row and close.
        let old_id;
        {
            let mut manifest = test_manifest();
            manifest.entities[0].crdt = false;
            let rt = Runtime::open(db, manifest).unwrap();
            old_id = rt
                .insert(
                    "User",
                    &serde_json::json!({"email": "a@b.com", "displayName": "Alice Zephyr"}),
                )
                .unwrap();
        }

        // Simulate `pylon migrate` adding the new column — plus the trigger
        // loss that comes with a SQLite table-rebuild migration (dropping or
        // renaming a table drops its triggers).
        {
            let conn = Connection::open(db).unwrap();
            conn.execute("ALTER TABLE \"User\" ADD COLUMN \"bio\" TEXT", [])
                .unwrap();
            for t in ["User_fts_ai", "User_fts_ad", "User_fts_au"] {
                conn.execute(&format!("DROP TRIGGER IF EXISTS \"{t}\""), [])
                    .unwrap();
            }
        }

        // v2: the manifest now declares `bio`. Boot must reconcile the FTS
        // table with the new text-field set.
        let mut manifest = test_manifest();
        manifest.entities[0].crdt = false;
        manifest.entities[0].fields.push(ManifestField {
            name: "bio".into(),
            field_type: "string".into(),
            optional: true,
            unique: false,
            crdt: None,
            server_only: false,
            readonly: false,
            default: None,
            enum_values: None,
            encrypted: false,
            sync_omit: false,
        });
        let rt = Runtime::open(db, manifest).unwrap();

        // Pre-fix this insert crashed: the recreated AFTER INSERT trigger
        // wrote `bio` into an FTS table that didn't have the column.
        rt.insert(
            "User",
            &serde_json::json!({
                "email": "b@c.com",
                "displayName": "Bob",
                "bio": "quixotic wanderer",
            }),
        )
        .unwrap();

        // The new text field is searchable...
        let hits = rt
            .query_filtered("User", &serde_json::json!({"$search": "quixotic"}))
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["email"], "b@c.com");
        // ...and the rebuild backfilled the row written under the old schema.
        let hits = rt
            .query_filtered("User", &serde_json::json!({"$search": "Zephyr"}))
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["id"], serde_json::json!(old_id));
    }

    /// SQLite stores bool columns as INTEGER 0/1. Reads MUST map them back
    /// to JSON booleans per the schema: raw 0/1 breaks every typed client
    /// (Swift's `0 → Bool` decode throws, so rows silently vanish from
    /// generated-struct queries; strict JS `=== false` checks fail too).
    #[test]
    fn bool_fields_read_back_as_json_booleans_not_ints() {
        let mut manifest = test_manifest();
        manifest.entities[0].fields = vec![
            ManifestField {
                name: "email".into(),
                field_type: "string".into(),
                optional: false,
                unique: true,
                crdt: None,
                server_only: false,
                readonly: false,
                default: None,
                enum_values: None,
                encrypted: false,
                sync_omit: false,
            },
            ManifestField {
                name: "isWarmup".into(),
                field_type: "bool".into(),
                optional: false,
                unique: false,
                crdt: None,
                server_only: false,
                readonly: false,
                default: None,
                enum_values: None,
                encrypted: false,
                sync_omit: false,
            },
            ManifestField {
                name: "isCompleted".into(),
                field_type: "bool".into(),
                optional: false,
                unique: false,
                crdt: None,
                server_only: false,
                readonly: false,
                default: None,
                enum_values: None,
                encrypted: false,
                sync_omit: false,
            },
            ManifestField {
                name: "reps".into(),
                field_type: "int".into(),
                optional: true,
                unique: false,
                crdt: None,
                server_only: false,
                readonly: false,
                default: None,
                enum_values: None,
                encrypted: false,
                sync_omit: false,
            },
        ];
        manifest.entities[0].crdt = false;
        let rt = Runtime::in_memory(manifest).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({
                    "email": "b@c.com",
                    "isWarmup": false,
                    "isCompleted": true,
                    "reps": 5,
                }),
            )
            .unwrap();

        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(
            row["isWarmup"],
            serde_json::Value::Bool(false),
            "bool false must read back as JSON false, not 0 — got {:?}",
            row["isWarmup"]
        );
        assert_eq!(row["isCompleted"], serde_json::Value::Bool(true));
        // Ints stay numbers — only schema-typed bools convert.
        assert_eq!(row["reps"], serde_json::Value::Number(5.into()));

        let listed = rt.list("User").unwrap();
        let listed_row = listed
            .iter()
            .find(|r| r["id"] == serde_json::Value::String(id.clone()))
            .unwrap();
        assert_eq!(listed_row["isCompleted"], serde_json::Value::Bool(true));
    }

    /// CRDT-mode entities (the default) populate the sidecar snapshot
    /// table on every write — the LoroDoc is the source of truth, the
    /// SQLite row is the materialized projection. This proves the CRDT
    /// branch in `insert` actually fires.
    #[test]
    fn crdt_default_writes_through_loro_store() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "x@y.com", "displayName": "Eric"}),
            )
            .unwrap();

        // Sidecar contains exactly one snapshot for the new row.
        let conn = rt.lock_write_conn().unwrap();
        let snap_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _pylon_crdt_snapshots
                 WHERE entity = ?1 AND row_id = ?2",
                rusqlite::params!["User", &id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(snap_count, 1, "sidecar should have one row after insert");

        // Loro doc is cached in memory after the write — proves
        // get_or_hydrate ran during apply_patch.
        assert!(rt.crdt_store().cached_rows() >= 1);

        // SQLite materialized view has the projected row.
        drop(conn);
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["email"], "x@y.com");
        assert_eq!(row["displayName"], "Eric");
    }

    /// Updates write through the LoroDoc as well — verifies the sidecar
    /// snapshot grows (Loro tracks new ops) and the materialized row
    /// reflects the new value.
    #[test]
    fn crdt_update_persists_new_snapshot() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "x@y.com", "displayName": "Eric"}),
            )
            .unwrap();

        let snap_after_insert: Vec<u8> = {
            let conn = rt.lock_write_conn().unwrap();
            conn.query_row(
                "SELECT snapshot FROM _pylon_crdt_snapshots
                 WHERE entity = 'User' AND row_id = ?1",
                rusqlite::params![&id],
                |r| r.get(0),
            )
            .unwrap()
        };

        rt.update("User", &id, &serde_json::json!({"displayName": "Eric C"}))
            .unwrap();

        let snap_after_update: Vec<u8> = {
            let conn = rt.lock_write_conn().unwrap();
            conn.query_row(
                "SELECT snapshot FROM _pylon_crdt_snapshots
                 WHERE entity = 'User' AND row_id = ?1",
                rusqlite::params![&id],
                |r| r.get(0),
            )
            .unwrap()
        };

        assert_ne!(
            snap_after_insert, snap_after_update,
            "snapshot bytes should change after an update"
        );

        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["displayName"], "Eric C");
        assert_eq!(row["email"], "x@y.com");
    }

    /// Regression: the transaction-bound insert path (`insert_with_conn`, which
    /// sits behind EVERY server-function / action `ctx.db.insert`) must seed the
    /// row's LoroDoc the same way `Runtime::insert` does. Before the fix it did
    /// a raw SQL insert with no CRDT seeding, so a row created by a server
    /// function had an EMPTY LoroDoc — and the first `/api/crdt` update projected
    /// NULLs over every non-CRDT column (foreign keys, tenant scope, …), failing
    /// NOT NULL constraints and clobbering data. Asserting a sidecar snapshot
    /// exists after `insert_with_conn` fails loudly if the seeding regresses.
    #[test]
    fn insert_with_conn_seeds_crdt_sidecar() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = {
            let conn = rt.lock_write_conn().unwrap();
            with_write_tx(&conn, || {
                rt.insert_with_conn(
                    &conn,
                    "User",
                    &serde_json::json!({"email": "tx@y.com", "displayName": "Tx"}),
                )
            })
            .unwrap()
        };

        // A CRDT snapshot exists for the row → the LoroDoc was seeded in the
        // same transaction. Pre-fix this count was 0.
        let conn = rt.lock_write_conn().unwrap();
        let snap_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _pylon_crdt_snapshots
                 WHERE entity = 'User' AND row_id = ?1",
                rusqlite::params![&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            snap_count, 1,
            "insert_with_conn must seed the CRDT sidecar (one snapshot row)"
        );

        // And the materialized row carries every field the seed captured.
        drop(conn);
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["email"], "tx@y.com");
        assert_eq!(row["displayName"], "Tx");
    }

    /// Regression: when the SQL INSERT step inside Runtime::insert fails
    /// (UNIQUE-constraint violation here), the LoroDoc snapshot must
    /// also roll back — neither half lands. Previously the LoroStore
    /// wrote first and committed independently, so a doomed INSERT left
    /// a sidecar row pointing at a doc that the materialized table
    /// never knew about.
    #[test]
    fn crdt_insert_rolls_back_when_sql_step_fails() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        // Seed a row.
        rt.insert(
            "User",
            &serde_json::json!({"email": "x@y.com", "displayName": "First"}),
        )
        .unwrap();

        // Snapshot the sidecar row count BEFORE the failing insert.
        let snap_count_before: i64 = {
            let conn = rt.lock_write_conn().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM _pylon_crdt_snapshots WHERE entity = 'User'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };

        // Attempt a duplicate-email insert. SQL UNIQUE rejects.
        let err = rt
            .insert(
                "User",
                &serde_json::json!({"email": "x@y.com", "displayName": "Second"}),
            )
            .expect_err("duplicate email must fail");
        assert_eq!(err.code, "INSERT_FAILED");

        // Sidecar row count unchanged — the LoroDoc snapshot the CRDT
        // path wrote was rolled back along with the failed SQL INSERT.
        let snap_count_after: i64 = {
            let conn = rt.lock_write_conn().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM _pylon_crdt_snapshots WHERE entity = 'User'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            snap_count_after, snap_count_before,
            "failed insert should not leave a sidecar snapshot behind"
        );
    }

    /// Entities with `crdt: false` skip the LoroDoc entirely — no sidecar
    /// row, no Loro cache entry. Proves the opt-out actually opts out.
    #[test]
    fn crdt_false_skips_loro_store() {
        let mut manifest = test_manifest();
        // Flip the User entity to LWW-only mode.
        manifest.entities[0].crdt = false;
        let rt = Runtime::in_memory(manifest).unwrap();

        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "lww@example.com", "displayName": "Plain"}),
            )
            .unwrap();

        let conn = rt.lock_write_conn().unwrap();
        let snap_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM _pylon_crdt_snapshots
                 WHERE entity = 'User' AND row_id = ?1",
                rusqlite::params![&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(snap_count, 0, "crdt:false should not touch the sidecar");
        assert_eq!(
            rt.crdt_store().cached_rows(),
            0,
            "crdt:false should not warm the cache"
        );

        // SQLite path still works — the row landed via the legacy
        // direct-write path.
        drop(conn);
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["email"], "lww@example.com");
    }

    #[test]
    fn list_entities() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        rt.insert(
            "User",
            &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
        )
        .unwrap();
        rt.insert(
            "User",
            &serde_json::json!({"email": "b@c.com", "displayName": "B"}),
        )
        .unwrap();
        let rows = rt.list("User").unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn update_entity() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
            )
            .unwrap();
        let updated = rt
            .update("User", &id, &serde_json::json!({"displayName": "Updated"}))
            .unwrap();
        assert!(updated);
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["displayName"], "Updated");
    }

    #[test]
    fn delete_entity() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
            )
            .unwrap();
        let deleted = rt.delete("User", &id).unwrap();
        assert!(deleted);
        assert!(rt.get_by_id("User", &id).unwrap().is_none());
    }

    #[test]
    fn lookup_by_field() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        rt.insert(
            "User",
            &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
        )
        .unwrap();
        let row = rt.lookup("User", "email", "a@b.com").unwrap().unwrap();
        assert_eq!(row["displayName"], "A");
    }

    #[test]
    fn unknown_entity_returns_error() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let err = rt.list("Nonexistent").unwrap_err();
        assert_eq!(err.code, "ENTITY_NOT_FOUND");
    }

    #[test]
    fn insert_rejects_unknown_column() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let err = rt
            .insert(
                "User",
                &serde_json::json!({"email": "a@b.com", "displayName": "A", "evil_col": "x"}),
            )
            .unwrap_err();
        assert_eq!(err.code, "INVALID_COLUMN");
    }

    #[test]
    fn update_rejects_unknown_column() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
            )
            .unwrap();
        let err = rt
            .update("User", &id, &serde_json::json!({"bad_field": "x"}))
            .unwrap_err();
        assert_eq!(err.code, "INVALID_COLUMN");
    }

    #[test]
    fn lookup_rejects_unknown_column() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let err = rt.lookup("User", "nonexistent", "val").unwrap_err();
        assert_eq!(err.code, "INVALID_COLUMN");
    }

    #[test]
    fn query_filtered_rejects_unknown_column() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let err = rt
            .query_filtered("User", &serde_json::json!({"bad_col": "x"}))
            .unwrap_err();
        assert_eq!(err.code, "INVALID_COLUMN");
    }

    #[test]
    fn query_filtered_rejects_unknown_order_column() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let err = rt
            .query_filtered("User", &serde_json::json!({"$order": {"bad_col": "asc"}}))
            .unwrap_err();
        assert_eq!(err.code, "INVALID_COLUMN");
    }

    #[test]
    fn query_filtered_sanitizes_order_direction() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        rt.insert(
            "User",
            &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
        )
        .unwrap();
        // Even a malicious direction value should be normalized to ASC.
        let rows = rt
            .query_filtered(
                "User",
                &serde_json::json!({"$order": {"email": "DROP TABLE User"}}),
            )
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn query_filtered_and_graph_decrypt_encrypted_fields_like_get() {
        // #353: a server-side ctx.db.query() / queryGraph() must return
        // PLAINTEXT for field.encrypted() columns, exactly like ctx.db.get().
        // Before the fix query_filtered skipped read normalization and returned
        // the raw `enc:v1:...` ciphertext — so a function that queried then
        // read a "decrypted" SSN got garbage. CI runs --test-threads=1, so
        // mutating the key env here is safe.
        std::env::set_var(
            "PYLON_ENCRYPTION_KEY",
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        let mk_field = |name: &str, server_only: bool, encrypted: bool| ManifestField {
            name: name.into(),
            field_type: "string".into(),
            optional: false,
            unique: false,
            crdt: None,
            server_only,
            readonly: false,
            default: None,
            enum_values: None,
            encrypted,
            sync_omit: false,
        };
        let manifest = AppManifest {
            manifest_version: 1,
            name: "Test".into(),
            version: "0.1.0".into(),
            entities: vec![pylon_kernel::ManifestEntity {
                name: "Vault".into(),
                // encrypted() requires serverOnly() (validated at boot).
                fields: vec![
                    mk_field("label", false, false),
                    mk_field("secret", true, true),
                ],
                indexes: vec![],
                relations: vec![],
                search: None,
                crdt: false,
                sync: true,
                ..Default::default()
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        };
        let rt = Runtime::in_memory(manifest).unwrap();
        // Guard against a vacuous pass: encryption must actually be configured.
        assert!(
            rt.encrypted_fields
                .get("Vault")
                .map(|v| v.iter().any(|f| f == "secret"))
                .unwrap_or(false),
            "Vault.secret must be a configured encrypted field"
        );

        let id = rt
            .insert(
                "Vault",
                &serde_json::json!({"label": "l1", "secret": "123-45-6789"}),
            )
            .unwrap();

        // Baseline: get_by_id already decrypts.
        let got = rt.get_by_id("Vault", &id).unwrap().unwrap();
        assert_eq!(got["secret"], "123-45-6789");

        // The fix: query_filtered must decrypt too (was leaking ciphertext).
        let rows = rt
            .query_filtered("Vault", &serde_json::json!({"label": "l1"}))
            .unwrap();
        assert_eq!(rows.len(), 1);
        let secret = rows[0]["secret"].as_str().unwrap();
        assert_eq!(
            secret, "123-45-6789",
            "query_filtered must return plaintext"
        );
        assert!(
            !secret.starts_with("enc:"),
            "query_filtered must not leak ciphertext: {secret}"
        );

        // query_graph routes through query_filtered → decrypts now too.
        let graph = rt
            .query_graph(&serde_json::json!({"Vault": {"where": {"label": "l1"}}}))
            .unwrap();
        assert_eq!(graph["Vault"][0]["secret"], "123-45-6789");

        std::env::remove_var("PYLON_ENCRYPTION_KEY");
    }

    #[test]
    fn manifest_arc_shares_one_allocation_not_a_deep_clone() {
        // #353: the transactional-function path takes an owned
        // Arc<AppManifest> via manifest_arc(). It must hand out the SAME
        // allocation (a refcount bump), not a fresh deep clone of every
        // entity/field/policy — two handles point at one buffer.
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        let a = rt.manifest_arc();
        let b = rt.manifest_arc();
        assert!(
            Arc::ptr_eq(&a, &b),
            "manifest_arc must share the allocation, not deep-clone"
        );
        // And it observes the same data the borrowing accessor returns.
        assert_eq!(a.entities.len(), rt.manifest().entities.len());
    }

    #[test]
    fn query_graph_include_batches_and_is_correct_across_parents() {
        // #349: include expansion fetches children in ONE batched `$in` query
        // per relation (not one per parent) and must still produce the right
        // nested results. Two parents — a many-relation with DIFFERENT bucket
        // sizes (2 vs 1) proves per-parent bucketing isn't cross-assigned; a
        // one-relation by id proves id-keyed assignment.
        use pylon_kernel::{ManifestEntity, ManifestField, ManifestRelation};
        let f = |name: &str| ManifestField {
            name: name.into(),
            field_type: "string".into(),
            optional: false,
            unique: false,
            crdt: None,
            server_only: false,
            readonly: false,
            default: None,
            enum_values: None,
            encrypted: false,
            sync_omit: false,
        };
        let ent = |name: &str, fields: Vec<ManifestField>, relations: Vec<ManifestRelation>| {
            ManifestEntity {
                name: name.into(),
                fields,
                indexes: vec![],
                relations,
                search: None,
                crdt: false,
                sync: true,
                ..Default::default()
            }
        };
        let manifest = AppManifest {
            manifest_version: 1,
            name: "G".into(),
            version: "0".into(),
            entities: vec![
                ent(
                    "Parent",
                    vec![f("groupKey"), f("ownerId")],
                    vec![
                        ManifestRelation {
                            name: "members".into(),
                            target: "Child".into(),
                            field: "groupKey".into(),
                            many: true,
                        },
                        ManifestRelation {
                            name: "owner".into(),
                            target: "Owner".into(),
                            field: "ownerId".into(),
                            many: false,
                        },
                    ],
                ),
                ent("Child", vec![f("groupKey"), f("label")], vec![]),
                ent("Owner", vec![f("ownerName")], vec![]),
            ],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        };
        let rt = Runtime::in_memory(manifest).unwrap();
        let o1 = rt
            .insert("Owner", &serde_json::json!({"ownerName": "Alice"}))
            .unwrap();
        let o2 = rt
            .insert("Owner", &serde_json::json!({"ownerName": "Bob"}))
            .unwrap();
        // group A has two children, group B has one.
        rt.insert(
            "Child",
            &serde_json::json!({"groupKey": "A", "label": "a1"}),
        )
        .unwrap();
        rt.insert(
            "Child",
            &serde_json::json!({"groupKey": "A", "label": "a2"}),
        )
        .unwrap();
        rt.insert(
            "Child",
            &serde_json::json!({"groupKey": "B", "label": "b1"}),
        )
        .unwrap();
        rt.insert(
            "Parent",
            &serde_json::json!({"groupKey": "A", "ownerId": &o1}),
        )
        .unwrap();
        rt.insert(
            "Parent",
            &serde_json::json!({"groupKey": "B", "ownerId": &o2}),
        )
        .unwrap();

        let graph = rt
            .query_graph(&serde_json::json!({
                "Parent": { "include": { "members": {}, "owner": {} } }
            }))
            .unwrap();
        let parents = graph["Parent"].as_array().unwrap();
        assert_eq!(parents.len(), 2);
        let p_a = parents.iter().find(|p| p["groupKey"] == "A").unwrap();
        let p_b = parents.iter().find(|p| p["groupKey"] == "B").unwrap();

        // many: A's bucket has 2 members, B's has exactly 1 — no cross-bleed.
        assert_eq!(p_a["members"].as_array().unwrap().len(), 2);
        assert_eq!(p_b["members"].as_array().unwrap().len(), 1);
        assert_eq!(p_b["members"][0]["label"], "b1");
        // one: each parent's owner resolved by id.
        assert_eq!(p_a["owner"]["ownerName"], "Alice");
        assert_eq!(p_b["owner"]["ownerName"], "Bob");
    }

    #[test]
    fn in_memory_has_no_read_pool() {
        let rt = Runtime::in_memory(test_manifest()).unwrap();
        assert_eq!(rt.read_pool_size(), 0);
    }

    #[test]
    fn open_creates_read_pool() {
        let dir = std::env::temp_dir().join(format!("pylon_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test_read_pool.db");

        let rt = Runtime::open(db_path.to_str().unwrap(), test_manifest()).unwrap();
        assert_eq!(rt.read_pool_size(), READ_POOL_SIZE);

        // Write then read through the pool.
        let id = rt
            .insert(
                "User",
                &serde_json::json!({"email": "pool@test.com", "displayName": "Pool"}),
            )
            .unwrap();
        let row = rt.get_by_id("User", &id).unwrap().unwrap();
        assert_eq!(row["email"], "pool@test.com");

        // Clean up.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_reads_dont_block_on_write() {
        use std::sync::Arc;

        let dir = std::env::temp_dir().join(format!("pylon_conc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test_concurrent.db");

        let rt = Arc::new(Runtime::open(db_path.to_str().unwrap(), test_manifest()).unwrap());

        // Seed some data so reads have something to return.
        rt.insert(
            "User",
            &serde_json::json!({"email": "a@b.com", "displayName": "A"}),
        )
        .unwrap();
        rt.insert(
            "User",
            &serde_json::json!({"email": "b@c.com", "displayName": "B"}),
        )
        .unwrap();

        // Hold the write lock to simulate a long write.
        let write_guard = rt.lock_write_conn().unwrap();

        // Spawn reader threads that should succeed despite the held write lock.
        let mut handles = Vec::new();
        for _ in 0..4 {
            let rt_clone = Arc::clone(&rt);
            handles.push(std::thread::spawn(move || {
                let rows = rt_clone.list("User").unwrap();
                assert_eq!(rows.len(), 2);
            }));
        }

        for h in handles {
            h.join().expect("reader thread panicked");
        }

        // Release the write lock.
        drop(write_guard);

        // Clean up.
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- accept_tcp (#345: macOS dual-stack accept panic) -----------------

    #[test]
    fn accept_tcp_yields_a_live_stream_and_peer_ip() {
        use std::io::{Read, Write};
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut s = std::net::TcpStream::connect(addr).unwrap();
            s.write_all(b"hi").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(50));
        });

        let (mut stream, ip) = accept_tcp(&listener).expect("accept");
        // The peer IP is decoded (loopback), not dropped.
        assert!(
            ip.is_some(),
            "peer IP should decode for a normal connection"
        );
        assert!(ip.unwrap().is_loopback());
        // And it's a real, readable connection.
        let mut buf = [0u8; 2];
        stream.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hi");
        client.join().unwrap();
    }

    /// The fix's core guarantee: an accept loop built on `accept_tcp` keeps
    /// running through a burst of connections that abort immediately — the
    /// exact churn that made libstd's asserting accept panic on macOS
    /// dual-stack `[::]`. We can't deterministically force the kernel to
    /// return a truncated sockaddr, but `accept_tcp` never parses one, so the
    /// loop must survive the burst and still serve a final good client.
    #[test]
    fn accept_tcp_loop_survives_aborted_connections() {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        // Dual-stack bind — the configuration that triggered the crash.
        let listener = bind_dual_stack_tcp(0).expect("dual-stack bind");
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));

        let stop_srv = Arc::clone(&stop);
        let server = std::thread::spawn(move || {
            // Mirror the production loops: accept_tcp + best-effort IP, never
            // panicking on a bad address.
            while !stop_srv.load(Ordering::Relaxed) {
                match accept_tcp(&listener) {
                    Ok((mut stream, _ip)) => {
                        let mut buf = [0u8; 4];
                        if stream
                            .read(&mut buf)
                            .map(|n| &buf[..n] == b"ping")
                            .unwrap_or(false)
                        {
                            let _ = stream.write_all(b"pong");
                        }
                    }
                    Err(_) => {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                }
            }
        });

        // Burst of connect-then-immediately-drop clients — the dead-on-arrival
        // churn behind the macOS sockaddr truncation that panicked the old
        // accept path.
        for _ in 0..50 {
            if let Ok(s) = std::net::TcpStream::connect(addr) {
                drop(s);
            }
        }

        // The loop must still be alive: a real client round-trips.
        let mut good = std::net::TcpStream::connect(addr).expect("good connect");
        good.write_all(b"ping").unwrap();
        let mut resp = [0u8; 4];
        good.read_exact(&mut resp)
            .expect("server still serving after the burst");
        assert_eq!(&resp, b"pong");

        stop.store(true, Ordering::Relaxed);
        // Unblock the server's accept() so the thread can observe `stop`.
        let _ = std::net::TcpStream::connect(addr);
        server.join().unwrap();
    }
}
