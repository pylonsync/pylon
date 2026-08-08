use std::fmt;

use serde::{Deserialize, Serialize};

pub mod clock;
pub mod errors;
pub mod net_guard;
pub mod studio;
pub mod util;

pub use clock::{Clock, MockClock, SystemClock};
pub use studio::StudioConfig;

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
    /// App-level LLM configuration. Selects the provider + default
    /// model used by `ctx.llm.complete` and `/api/llm/complete` when
    /// the env doesn't already pin them.
    ///
    /// Defaults to `LlmProviderName::None` — apps that don't declare
    /// an `llm({...})` block get the missing-config gap surface
    /// (`LLM_NOT_CONFIGURED`) instead of silently no-op'ing.
    #[serde(default, skip_serializing_if = "ManifestLlmConfig::is_default")]
    pub llm: ManifestLlmConfig,
    /// External OAuth integrations declared via `defineConnection({...})`.
    /// Each entry adds a `ctx.connections.<name>` surface for
    /// server-side actions to fetch OAuth-protected resources on
    /// behalf of the signed-in user. Token storage uses the auto-
    /// created `_Connection` entity (AEAD-encrypted at rest).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ManifestConnection>,
    /// Recurring jobs declared via the SDK's `cron(schedule, functionName)`
    /// helper in app.ts. At boot the runtime registers each with the cron
    /// scheduler, firing the named function every time the schedule matches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crons: Vec<ManifestCron>,
    /// Self-hosted web fonts declared via the SDK's `font({...})` helper.
    /// At build the runtime fetches each family from Google Fonts, self-hosts
    /// the woff2 same-origin, generates `@font-face` + a size-adjusted fallback
    /// face (zero layout shift), and auto-injects the preload + faces into every
    /// SSR page's `<head>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fonts: Vec<ManifestFont>,
}

/// One recurring job: run `function` on the `schedule` (a 5-field cron
/// expression). Emitted by the SDK's `cron(...)` helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestCron {
    /// 5-field cron expression, e.g. `"0 * * * *"`.
    pub schedule: String,
    /// Name of the function (query/mutation/action) to run.
    pub function: String,
    /// Optional human description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One self-hosted web font. Emitted by the SDK's `font({...})` helper.
/// The build fetches the family, self-hosts the woff2 same-origin, and
/// generates the `@font-face` + size-adjusted fallback faces from the file's
/// real sfnt metrics so the swap-in produces no layout shift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFont {
    /// Google Fonts family name, e.g. "Geist" or "Inter".
    pub family: String,
    /// CSS custom property the app applies the font through, e.g.
    /// "--font-sans". Resolves to the family + the adjusted fallback +
    /// the `fallback` stack.
    pub variable: String,
    /// Weights to load — specific (`["400","700"]`) or a CSS2 range
    /// (`["300..700"]`).
    #[serde(default)]
    pub weights: Vec<String>,
    /// Styles to load (`"normal"` / `"italic"`).
    #[serde(default)]
    pub styles: Vec<String>,
    /// Unicode subsets, e.g. `["latin"]`.
    #[serde(default)]
    pub subsets: Vec<String>,
    /// CSS `font-display`. Defaults to `"swap"`.
    #[serde(default)]
    pub display: Option<String>,
    /// Emit `<link rel="preload">` for the font files. Defaults to `true`.
    #[serde(default = "default_true")]
    pub preload: bool,
    /// Fallback stack appended after the family + adjusted fallback face.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback: Vec<String>,
    /// Generate a size-adjusted fallback `@font-face` for zero layout shift.
    /// Defaults to `true`.
    #[serde(default = "default_true")]
    pub adjust_font_fallback: bool,
}

fn default_true() -> bool {
    true
}

/// One external OAuth integration. Emitted by the SDK's
/// `defineConnection({...})` factory in app.ts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestConnection {
    /// App-facing key — `ctx.connections.get("google")` matches on
    /// this. Different connections can target the same provider
    /// with different scopes (e.g. "google-calendar" vs "google-drive").
    pub name: String,
    /// Provider identifier the runtime's OAuth client understands —
    /// matches `pylon-auth`'s built-in provider list (`google`,
    /// `github`, `slack`, `microsoft`, `notion`, etc.) or an
    /// `oidc:` prefix for generic OIDC discovery.
    pub provider: String,
    /// Whitespace-separated scopes. Empty = the provider's default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scopes: String,
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
    /// Org / membership / invite entity configuration. Apps that want
    /// org features declare the three entities in their schema and
    /// (optionally) override the entity names here. Default names are
    /// `Org`, `OrgMember`, `OrgInvite` — apps following the convention
    /// get zero-config.
    #[serde(default)]
    pub org: ManifestAuthOrgConfig,
    /// Per-app trusted origins for OAuth `?callback=` validation.
    /// Merged with anything in `PYLON_TRUSTED_ORIGINS` env.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_origins: Vec<String>,
}

/// Org / OrgMember / OrgInvite entity configuration.
///
/// Pylon's `/api/auth/orgs/*` surface reads + writes through the
/// manifest's entity layer — apps declare these three entities in their
/// schema and add whatever extra fields they need (logo, industry,
/// billing email, etc.) without touching the framework.
///
/// Required fields per entity (the framework reads/writes these by name):
///
/// **Org** — `id`, `name`, `createdBy`, `createdAt`
/// **OrgMember** — `id`, `orgId`, `userId`, `role`, `joinedAt`
/// **OrgInvite** — `id`, `orgId`, `email`, `role`, `invitedBy`,
///                `tokenHash`, `tokenPrefix`, `createdAt`, `expiresAt`,
///                `acceptedAt` (optional)
///
/// Apps free to add any other field. Reads return the full row;
/// writes only set framework-managed fields and leave the rest alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestAuthOrgConfig {
    /// Manifest entity name for the Org table. Default `"Org"`.
    #[serde(default = "default_org_entity")]
    pub entity: String,
    /// Manifest entity name for the OrgMember (membership) table.
    /// Default `"OrgMember"`.
    #[serde(default = "default_member_entity")]
    pub member_entity: String,
    /// Manifest entity name for the OrgInvite table. Default
    /// `"OrgInvite"`.
    #[serde(default = "default_invite_entity")]
    pub invite_entity: String,
    /// When `true`, /api/auth/orgs/* endpoints return 501
    /// ORG_NOT_CONFIGURED. Apps that don't want the framework's org
    /// surface (e.g. they implement their own in TS) opt out here.
    #[serde(default)]
    pub disabled: bool,
}

impl Default for ManifestAuthOrgConfig {
    fn default() -> Self {
        Self {
            entity: default_org_entity(),
            member_entity: default_member_entity(),
            invite_entity: default_invite_entity(),
            disabled: false,
        }
    }
}

fn default_org_entity() -> String {
    "Org".into()
}
fn default_member_entity() -> String {
    "OrgMember".into()
}
fn default_invite_entity() -> String {
    "OrgInvite".into()
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
    /// Field name on the User row that, when truthy, marks the
    /// session as `auth.is_admin = true`. Default unset (only the
    /// PYLON_ADMIN_TOKEN env-bearer path grants admin). When set,
    /// resolving a session cookie loads the user, reads this field,
    /// and lifts is_admin if it's `true`/`1`/non-empty.
    ///
    /// Apps that want per-user admin (Studio access for specific
    /// User rows instead of a shared bootstrap token) set this to
    /// `"isAdmin"` (or whichever bool field they store on User).
    /// Pylon-cloud uses this so platform admins sign in with their
    /// regular account and Studio respects the role.
    ///
    /// Bootstrap pattern: PYLON_ADMIN_TOKEN keeps working for CI /
    /// fresh deploys with no User rows yet. The two paths are
    /// additive — admin token OR matching admin field both grant
    /// `is_admin`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_field: Option<String>,
}

impl Default for ManifestAuthUserConfig {
    fn default() -> Self {
        Self {
            entity: default_user_entity(),
            expose: Vec::new(),
            hide: Vec::new(),
            admin_field: None,
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

// ---------------------------------------------------------------------------
// LLM configuration
// ---------------------------------------------------------------------------

/// App-level LLM provider config. Emitted by the SDK's `llm({...})`
/// factory in app.ts. All fields optional; environment variables
/// (`PYLON_LLM_PROVIDER`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
/// `PYLON_LLM_MODEL`) take precedence so operators can override per
/// deploy without redeploying the app bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ManifestLlmConfig {
    /// Provider name. `None` = no manifest preference; the runtime
    /// falls back to env detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<LlmProviderName>,
    /// Default model name (e.g. `claude-sonnet-4-5`, `gpt-4o`). When
    /// set + the caller doesn't supply an explicit `model`, requests
    /// use this. Overridden by `PYLON_LLM_MODEL` env.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// Optional allowlist of models callers may request via the
    /// `model` field. Same gate as `PYLON_AI_MODELS_ALLOWED` env;
    /// the manifest list is merged with whatever env carries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_models: Vec<String>,
}

impl ManifestLlmConfig {
    pub fn is_default(&self) -> bool {
        self.provider.is_none() && self.default_model.is_none() && self.allowed_models.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmProviderName {
    Anthropic,
    Openai,
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
    /// Client replication. Default `true` — the entity is bulk-snapshotted +
    /// delta-streamed into every client's local replica. Set `false` for large,
    /// server-queried catalogs (a 10k-row product table) that the client reaches
    /// via search + by-id fetch instead of holding the whole table locally:
    /// `sync: false` excludes the entity from the snapshot AND the change-log
    /// delta, so it never floods the replica. Direct reads (`/api/entities/X`,
    /// `/api/search/X`) and policies are unchanged.
    #[serde(default = "default_sync_enabled")]
    pub sync: bool,
    /// Replication SCOPE — a policy-DSL predicate that bounds WHICH rows of a
    /// synced entity reach a given client's replica.
    ///
    /// `sync` is all-or-nothing: a client replicates every row it is allowed
    /// to read. That is fine for a working set and wrong for a table that
    /// grows without bound, where the only escape hatch was `sync: false` —
    /// giving up live queries entirely. A scope keeps the entity live while
    /// bounding the volume: `data.orgId == auth.tenantId`, or a retention
    /// window like `data.createdAt >= now - 2592000000`.
    ///
    /// Evaluated per row, per caller, in the SAME expression language as
    /// policies (including `exists(...)`), and applied ON TOP of the read
    /// policy — never instead of it. A scope is a bandwidth decision, not an
    /// access-control one; narrowing it can never widen what a caller sees.
    ///
    /// Applies to REPLICATION only — the snapshot, the cursor bootstrap, and
    /// the change-log delta. Direct reads (`/api/entities/X`, `/api/search/X`)
    /// are unchanged, matching `sync: false`'s contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_scope: Option<String>,
    /// Hard ceiling on rows replicated per entity, per client. Belt to
    /// `sync_scope`'s braces: a predicate that turns out not to bound growth
    /// (or one an app forgot to narrow) still can't flood a replica.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_limit: Option<usize>,
}

fn default_crdt_enabled() -> bool {
    true
}

fn default_sync_enabled() -> bool {
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
            sync: true,
            sync_scope: None,
            sync_limit: None,
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
    /// Tokenizer language for the FTS index (Postgres `to_tsvector` /
    /// `plainto_tsquery` config). Only the names Postgres ships in
    /// `pg_ts_config` are valid: `english`, `spanish`, `german`,
    /// `french`, `simple`, etc. SQLite ignores the field — its FTS5
    /// virtual table uses `unicode61 remove_diacritics 2` regardless,
    /// which is language-agnostic. Defaults to `english` so existing
    /// manifests don't change behavior.
    #[serde(default)]
    pub language: Option<String>,
}

impl ManifestSearchConfig {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.facets.is_empty() && self.sortable.is_empty()
    }

    /// Resolve the tsvector language config — caller's `language` if
    /// set, otherwise `english`. Only used by the Postgres backend;
    /// SQLite ignores it.
    pub fn language_or_default(&self) -> &str {
        self.language.as_deref().unwrap_or("english")
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
    /// `true` when the TS def used `field.X().serverOnly()`. The
    /// framework strips this field from every public HTTP response —
    /// entity list, entity get, session projection, sync push. Still
    /// readable from inside server functions via `ctx.db.*` so
    /// app code can use it (Stripe customer id, internal cache
    /// columns) without leaking.
    ///
    /// Defaults to `false` for backwards compatibility — apps that
    /// haven't annotated anything keep their pre-modifier shape.
    #[serde(default, rename = "serverOnly", skip_serializing_if = "is_false_ref")]
    pub server_only: bool,
    /// `true` when the TS def used `field.X().readonly()`. The
    /// framework rejects any HTTP update payload that mentions this
    /// field with a `READONLY_FIELD` error, before policy
    /// evaluation. Admin contexts bypass. Server-side writes via
    /// `ctx.db.update` are always allowed — readonly is an HTTP
    /// boundary check, not a hard write-lock.
    #[serde(default, rename = "readonly", skip_serializing_if = "is_false_ref")]
    pub readonly: bool,
    /// Default value to inject on insert when the row omits this field.
    /// - `"now"`     → runtime stamps the current UTC time
    /// - any literal → runtime stamps that exact value
    /// Maps to `field.X().defaultNow()` / `.default(value)` on the
    /// TS side. None = no default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// Allowed values when this field was declared as
    /// `field.enum([...])`. Empty / None means "no enum constraint" —
    /// the field is a plain string. Recorded so codegen + runtime
    /// can narrow the value type and (eventually) reject out-of-set
    /// inserts at the insert path.
    #[serde(
        default,
        rename = "enumValues",
        skip_serializing_if = "Option::is_none"
    )]
    pub enum_values: Option<Vec<String>>,
    /// `true` when the TS def used `field.X().encrypted()`. The
    /// framework AEAD-encrypts the value before SQLite/Postgres
    /// persistence and decrypts on read. The plaintext only exists
    /// in memory inside the Pylon process; a hot-copy of the DB file
    /// or a compromised SQL dump exposes only ciphertext.
    ///
    /// Backed by `PYLON_ENCRYPTION_KEY` (32-byte URL-safe base64 key).
    /// AEAD = ChaCha20-Poly1305. Per-cell random nonce stored alongside
    /// the ciphertext. Wire format: `enc:v1:<base64(nonce)>:<base64(ct)>`.
    ///
    /// Only valid for `string`/`text`/`json` field types. Booleans, ints,
    /// timestamps stay queryable + indexable. Indexed encrypted fields
    /// only support exact match via deterministic encryption (future
    /// work); for now, encrypted fields with `unique: true` are
    /// rejected at manifest validation.
    #[serde(default, rename = "encrypted", skip_serializing_if = "is_false_ref")]
    pub encrypted: bool,
    /// `true` when the TS def used `field.X().syncOmit()`. The field is
    /// stripped from every REPLICATION surface — snapshot pull, delta
    /// pull, WS change fanout, `?sync=1` cursor fetches — but stays on
    /// direct reads (entity get/list, plain cursor, queries, SSR
    /// serverData) and inside server functions.
    ///
    /// This is the "heavy but not secret" modifier: a large JSON/blob
    /// column (render plans, slide decks, generated markdown) shouldn't
    /// ride into every browser's replica on every sync, but the detail
    /// view that needs it can still fetch it by id. Contrast
    /// `serverOnly`, which hides a field from ALL client surfaces.
    ///
    /// Client replicas simply never hold the column; reconcile compares
    /// replicated rows against replication fetches, so the omission is
    /// symmetric and never reads as drift.
    #[serde(default, rename = "syncOmit", skip_serializing_if = "is_false_ref")]
    pub sync_omit: bool,
}

fn is_false_ref(b: &bool) -> bool {
    !*b
}

impl Default for ManifestField {
    /// Convenience for test fixtures — `ManifestField { name: ..., field_type: ...,
    /// ..Default::default() }`. The shipped manifest serialization
    /// never emits a default field; every `ManifestField` in the runtime
    /// path comes from `buildManifest` in @pylonsync/sdk.
    fn default() -> Self {
        Self {
            name: String::new(),
            field_type: "string".to_string(),
            optional: false,
            unique: false,
            crdt: None,
            server_only: false,
            readonly: false,
            default: None,
            enum_values: None,
            encrypted: false,
            sync_omit: false,
        }
    }
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

/// One attachment on an outgoing email. `content` is base64; the
/// provider layer forwards it verbatim. `content_type` passes through
/// UNTOUCHED — parameterized types like `text/calendar; method=REQUEST`
/// are load-bearing (they're what makes mail clients render an RSVP-able
/// calendar invite instead of a file download).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailAttachment {
    pub filename: String,
    pub content_type: String,
    /// Base64-encoded file bytes.
    pub content: String,
}

/// An outgoing email, shared by the app channel (`ctx.email.send`) and
/// the auth channel (verification / magic-code mail). One struct through
/// every layer — hook, router trait, transport — so adding a field is a
/// single-site change instead of a three-trait signature break.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    /// Plain-text body. Always present — providers use it as the
    /// text/plain part (and clients with HTML disabled fall back to it).
    pub text: String,
    /// Optional HTML body; providers send multipart with `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<EmailAttachment>,
}

impl EmailMessage {
    /// Plain-text message — what every auth call site sends.
    pub fn plain(to: &str, subject: &str, text: &str) -> Self {
        Self {
            to: to.to_string(),
            subject: subject.to_string(),
            text: text.to_string(),
            html: None,
            attachments: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestIndex {
    pub name: String,
    pub fields: Vec<String>,
    pub unique: bool,
    /// Optional SQL predicate that turns this into a *partial* index.
    /// Both SQLite and Postgres support `CREATE [UNIQUE] INDEX … WHERE`,
    /// which lets the same index enforce different cardinality rules
    /// for different rows — e.g. a unique constraint on
    /// `(createdBy)` only when `plan = 'hobby'`, so a single user can
    /// own multiple paid orgs but only one hobby org.
    ///
    /// Skipped on serialize when None so existing manifests stay
    /// byte-for-byte identical.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "where")]
    pub where_clause: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRoute {
    pub path: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
    /// Project-relative module path for SSR-rendered routes (e.g.
    /// `app/hello/page`). Populated by the file-system discoverer
    /// when scanning `app/**/page.tsx`. The Bun-side `@pylonsync/ssr`
    /// adapter dynamically imports this module and renders its
    /// default export via `renderToReadableStream`. Skipped on
    /// serialize for non-SSR routes so older manifests stay
    /// byte-for-byte identical.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// Layout chain: project-relative module paths walked from root
    /// down to leaf, each wrapping the next as children. Discovered
    /// from `app/.../layout.tsx` files on the route's path. Empty
    /// when no layouts apply. Skipped on serialize when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layouts: Vec<String>,
    /// Route kind. `None` (or `"page"`) is a normal navigable page.
    /// `"not-found"` / `"error"` are SSR boundary modules discovered
    /// from `app/**/not-found.tsx` and `app/**/error.tsx`. Boundary
    /// routes are NOT matched as navigable URLs — the SSR host consults
    /// them for unmatched-URL 404s (`not-found`) and render-failure 500s
    /// (`error`); the `path` records the segment prefix the boundary
    /// covers (`/` for root). Skipped on serialize when `None` so
    /// page-only manifests stay byte-for-byte identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
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
    fn manifest_deserializes_crons_from_sdk_shape() {
        // The manifest is built in TS (SDK `cron(schedule, fn)` → buildManifest)
        // and deserialized here. This pins the field names so a rename on
        // either side can't silently drop every app cron. Shape mirrors the
        // SDK's `{ schedule, function, description? }`.
        let json = r#"{
            "manifest_version": 1, "name": "t", "version": "0",
            "entities": [], "routes": [], "queries": [], "actions": [], "policies": [],
            "crons": [
              { "schedule": "0 * * * *", "function": "hourlyRollup" },
              { "schedule": "*/5 * * * *", "function": "pollInbox", "description": "poll" }
            ]
        }"#;
        let m: AppManifest = serde_json::from_str(json).expect("deserialize manifest with crons");
        assert_eq!(m.crons.len(), 2);
        assert_eq!(m.crons[0].schedule, "0 * * * *");
        assert_eq!(m.crons[0].function, "hourlyRollup");
        assert_eq!(m.crons[0].description, None);
        assert_eq!(m.crons[1].function, "pollInbox");
        assert_eq!(m.crons[1].description.as_deref(), Some("poll"));

        // A manifest with no `crons` key (older apps) defaults to empty.
        let no_crons = r#"{ "manifest_version": 1, "name": "t", "version": "0",
            "entities": [], "routes": [] }"#;
        let m2: AppManifest = serde_json::from_str(no_crons).expect("deserialize without crons");
        assert!(m2.crons.is_empty());
    }

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
