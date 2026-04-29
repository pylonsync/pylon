use std::fmt;

use serde::{Deserialize, Serialize};

pub mod clock;
pub mod errors;
pub mod util;

pub use clock::{Clock, MockClock, SystemClock};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Ok = 0,
    Error = 1,
    Usage = 64,
    Unavailable = 69,
}

impl ExitCode {
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

// ---------------------------------------------------------------------------
// Severity & Span — shared diagnostic primitives
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => f.write_str("error"),
            Severity::Warning => f.write_str("warning"),
            Severity::Info => f.write_str("info"),
        }
    }
}

/// Optional source location for a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

// ---------------------------------------------------------------------------
// Diagnostic — structured, machine-readable error/warning
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.severity, self.code, self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, " (hint: {hint})")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// AppManifest — canonical manifest shape
// ---------------------------------------------------------------------------

pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AppManifest {
    pub manifest_version: u32,
    pub name: String,
    pub version: String,
    pub entities: Vec<ManifestEntity>,
    pub routes: Vec<ManifestRoute>,
    #[serde(default)]
    pub queries: Vec<ManifestQuery>,
    #[serde(default)]
    pub actions: Vec<ManifestAction>,
    #[serde(default)]
    pub policies: Vec<ManifestPolicy>,
    /// App-level auth configuration. Mirrors better-auth's
    /// `betterAuth({ user, session, trustedOrigins })` shape — controls
    /// the manifest entity name pylon treats as the User table, which
    /// fields get exposed via `/api/auth/session`, the cookie claims
    /// cache, and per-app trusted origins.
    ///
    /// Defaults are sensible (`User` entity, hide `passwordHash`,
    /// 30-day sessions, no cookie cache, trusted-origins from
    /// `PYLON_TRUSTED_ORIGINS` env) so apps that don't define an
    /// `auth({...})` block in app.ts still work.
    #[serde(default)]
    pub auth: ManifestAuthConfig,
}

/// Pylon's auth configuration block — emitted by the SDK's
/// `auth({...})` factory in app.ts. All fields optional; missing
/// values fall back to framework defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ManifestAuthConfig {
    #[serde(default)]
    pub user: ManifestAuthUserConfig,
    #[serde(default)]
    pub session: ManifestAuthSessionConfig,
    /// Per-app trusted origins for OAuth `?callback=` validation.
    /// Merged with anything in `PYLON_TRUSTED_ORIGINS` env.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_origins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestAuthUserConfig {
    /// Manifest entity name pylon treats as the User table.
    /// Default `"User"` — the convention every existing pylon app
    /// already follows.
    #[serde(default = "default_user_entity")]
    pub entity: String,
    /// Optional allowlist of fields exposed via `/api/auth/session`.
    /// When set, ONLY these fields appear in the response (`id` is
    /// always included). Useful for apps that want strict schemas.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expose: Vec<String>,
    /// Additional fields to strip from the User row before responding.
    /// Combined with the framework defaults (`passwordHash` plus
    /// anything starting with `_`). Use this for app-specific
    /// secrets stored on the User row.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
}

impl Default for ManifestAuthUserConfig {
    fn default() -> Self {
        Self {
            entity: default_user_entity(),
            expose: Vec::new(),
            hide: Vec::new(),
        }
    }
}

fn default_user_entity() -> String {
    "User".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestAuthSessionConfig {
    /// Lifetime of new sessions in seconds. Default 30 days.
    #[serde(default = "default_session_lifetime")]
    pub expires_in: u64,
    /// Cookie cache config — bakes the listed claims into the cookie
    /// itself so `/api/auth/me`-style probes can resolve identity
    /// without a session-store lookup. Mirrors better-auth's
    /// `session.cookieCache`.
    #[serde(default)]
    pub cookie_cache: ManifestAuthCookieCacheConfig,
}

impl Default for ManifestAuthSessionConfig {
    fn default() -> Self {
        Self {
            expires_in: default_session_lifetime(),
            cookie_cache: ManifestAuthCookieCacheConfig::default(),
        }
    }
}

fn default_session_lifetime() -> u64 {
    30 * 24 * 60 * 60
}

/// Cookie-cache settings. When `enabled`, the session cookie carries
/// a signed JWT-style envelope including the claims listed in
/// `claims` (defaults to `is_admin` + `tenant_id`). Cookie reads
/// resolve identity without touching the session store, at the cost
/// of staleness up to `max_age` seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestAuthCookieCacheConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Max age of the cached claims in seconds. After this, the
    /// cookie envelope is treated as expired and the session store
    /// is consulted again. Default 5 minutes — same as better-auth.
    #[serde(default = "default_cookie_cache_max_age")]
    pub max_age: u64,
    /// Auth-context fields baked into the cookie envelope. Always
    /// includes `user_id`; the operator opts in to anything else.
    #[serde(default = "default_cookie_cache_claims")]
    pub claims: Vec<String>,
}

impl Default for ManifestAuthCookieCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_age: default_cookie_cache_max_age(),
            claims: default_cookie_cache_claims(),
        }
    }
}

fn default_cookie_cache_max_age() -> u64 {
    5 * 60
}

fn default_cookie_cache_claims() -> Vec<String> {
    vec!["is_admin".into(), "tenant_id".into()]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntity {
    pub name: String,
    pub fields: Vec<ManifestField>,
    pub indexes: Vec<ManifestIndex>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<ManifestRelation>,
    /// Opt-in faceted search config. `None` = entity isn't searchable;
    /// `Some(cfg)` makes the runtime create FTS5 + facet-bitmap shadow
    /// tables on schema push and maintain them on every write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<ManifestSearchConfig>,
    /// Local-first / CRDT mode. Default `true` — every entity is backed
    /// by a Loro doc, mutations merge as CRDTs, multi-device offline
    /// edits converge cleanly. Set `false` to opt out per entity (audit
    /// logs, append-only archives, anything that doesn't need offline
    /// merge and where you want to skip the per-write Loro overhead).
    /// The SQLite-projected row shape is identical either way; queries
    /// and indexes don't change between modes.
    #[serde(default = "default_crdt_enabled")]
    pub crdt: bool,
}

fn default_crdt_enabled() -> bool {
    true
}

impl Default for ManifestEntity {
    fn default() -> Self {
        Self {
            name: String::new(),
            fields: Vec::new(),
            indexes: Vec::new(),
            relations: Vec::new(),
            search: None,
            crdt: true,
        }
    }
}

/// Per-entity search declaration. Lives on the manifest so both the
/// storage layer (schema push) and the runtime (write-time maintenance
/// + query endpoints) read the same shape.
///
/// Kept in `pylon-kernel` intentionally — other crates depend on kernel
/// but not on each other, so this is the only place every layer can
/// agree on the config surface.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestSearchConfig {
    #[serde(default)]
    pub text: Vec<String>,
    #[serde(default)]
    pub facets: Vec<String>,
    #[serde(default)]
    pub sortable: Vec<String>,
}

impl ManifestSearchConfig {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.facets.is_empty() && self.sortable.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRelation {
    pub name: String,
    pub target: String,
    pub field: String,
    #[serde(default)]
    pub many: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub optional: bool,
    pub unique: bool,
    /// CRDT container override for this field. `None` = pick a sensible
    /// default for the field type (most things are LWW; `richtext`
    /// defaults to LoroText). Typed enum so typos in the manifest
    /// fail at deserialize time instead of at first write.
    ///
    /// Ignored when the entity has `crdt: false` (the LWW-only escape
    /// hatch on the entity itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crdt: Option<CrdtAnnotation>,
}

/// Per-field CRDT container override. Wire format is the lowercase
/// kebab-case string each variant maps to (e.g. `"text"`, `"movable-list"`),
/// so JSON manifests look the same as before — but a typo like
/// `crdt: "txt"` now fails at manifest deserialization with a clear
/// "unknown variant" error instead of slipping through and erroring at
/// first write.
///
/// Variants intentionally mirror the categories
/// [`pylon_crdt::CrdtFieldKind`] knows how to instantiate. New CRDT
/// container types added to Loro show up as new variants here, plus a
/// match arm in `pylon_crdt::field_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CrdtAnnotation {
    /// Explicit LWW register (matches the default for most scalar types).
    Lww,
    /// Upgrade `string` → `LoroText` for collaborative character-level merge.
    Text,
    /// Upgrade `int`/`float` → `LoroCounter` so concurrent increments add
    /// instead of stomping. Reserved — apply_patch returns
    /// "not yet implemented" until the projection layer learns counters.
    Counter,
    /// `LoroList` for ordered collections. Reserved.
    List,
    /// `LoroMovableList` for reorderable lists (kanban, prioritized todo).
    /// Reserved.
    #[serde(rename = "movable-list")]
    MovableList,
    /// `LoroTree` for hierarchical data (folders, threaded comments).
    /// Reserved.
    Tree,
}

impl CrdtAnnotation {
    /// Wire-format string. Stable across versions; changing this breaks
    /// every persisted manifest on disk.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lww => "lww",
            Self::Text => "text",
            Self::Counter => "counter",
            Self::List => "list",
            Self::MovableList => "movable-list",
            Self::Tree => "tree",
        }
    }
}

impl std::fmt::Display for CrdtAnnotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestIndex {
    pub name: String,
    pub fields: Vec<String>,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRoute {
    pub path: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestQuery {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<ManifestField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestAction {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<ManifestField>,
}

/// Row-level access policy attached to an entity or action.
///
/// `allow` is the legacy single-gate expression used for every kind of
/// access. The optional `allow_*` fields let callers differentiate read
/// from write from delete. When a per-action field is present it wins;
/// otherwise the engine falls back to `allow`. That keeps old manifests
/// working unchanged while enabling finer-grained ownership rules —
/// "anyone can read, only the author can edit or delete."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ManifestPolicy {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub allow: String,
    /// Overrides `allow` for reads (pull, list, get). Optional.
    #[serde(default, rename = "allowRead", skip_serializing_if = "Option::is_none")]
    pub allow_read: Option<String>,
    /// Overrides `allow` for inserts. Optional; falls back to `allow_write`
    /// then `allow`.
    #[serde(
        default,
        rename = "allowInsert",
        skip_serializing_if = "Option::is_none"
    )]
    pub allow_insert: Option<String>,
    /// Overrides `allow`/`allow_write` for updates. Optional.
    #[serde(
        default,
        rename = "allowUpdate",
        skip_serializing_if = "Option::is_none"
    )]
    pub allow_update: Option<String>,
    /// Overrides `allow`/`allow_write` for deletes. Optional.
    #[serde(
        default,
        rename = "allowDelete",
        skip_serializing_if = "Option::is_none"
    )]
    pub allow_delete: Option<String>,
    /// Shared fallback for any write (insert/update/delete) when the
    /// more-specific field isn't set. Optional.
    #[serde(
        default,
        rename = "allowWrite",
        skip_serializing_if = "Option::is_none"
    )]
    pub allow_write: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_values() {
        assert_eq!(ExitCode::Ok.as_i32(), 0);
        assert_eq!(ExitCode::Error.as_i32(), 1);
        assert_eq!(ExitCode::Usage.as_i32(), 64);
        assert_eq!(ExitCode::Unavailable.as_i32(), 69);
    }

    #[test]
    fn severity_display() {
        assert_eq!(format!("{}", Severity::Error), "error");
        assert_eq!(format!("{}", Severity::Warning), "warning");
        assert_eq!(format!("{}", Severity::Info), "info");
    }

    #[test]
    fn diagnostic_display_without_hint() {
        let d = Diagnostic {
            severity: Severity::Error,
            code: "TEST".into(),
            message: "something failed".into(),
            span: None,
            hint: None,
        };
        assert_eq!(format!("{d}"), "[error] TEST: something failed");
    }

    #[test]
    fn diagnostic_display_with_hint() {
        let d = Diagnostic {
            severity: Severity::Warning,
            code: "WARN".into(),
            message: "check this".into(),
            span: None,
            hint: Some("try again".into()),
        };
        assert_eq!(
            format!("{d}"),
            "[warning] WARN: check this (hint: try again)"
        );
    }

    #[test]
    fn manifest_version_constant() {
        assert_eq!(MANIFEST_VERSION, 1);
    }
}
