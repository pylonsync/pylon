use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use pylon_auth::AuthContext;
use pylon_kernel::{AppManifest, ManifestPolicy};

// ---------------------------------------------------------------------------
// Policy evaluation
// ---------------------------------------------------------------------------

/// Adapter the policy engine uses to evaluate `exists(...)` predicates.
///
/// `exists()` lets a policy assert that some related row is present
/// before allowing access — the canonical use case is multi-tenant
/// membership checks:
///
/// ```text
/// allowRead: "exists(OrgMember where orgId == data.orgId and userId == auth.userId)"
/// ```
///
/// Implementing this requires the engine to actually query a row store
/// — which lives in the runtime / router crates, outside `pylon-policy`.
/// We define a narrow trait here so the policy crate stays a leaf
/// dependency and the runtime can wire its `DataStore` into evaluation
/// without dragging the whole storage layer into this crate's tree.
///
/// `entity_exists` returns `true` iff at least one row of `entity`
/// matches every `(field_path, value)` pair. The path is a dotted
/// path resolved against the row (e.g. `["orgId"]`, `["nested", "id"]`).
/// Values are JSON literals or strings/nulls.
///
/// Implementations should bound the lookup at a single row (existence
/// only), and SHOULD support both top-level fields and one-level nested
/// paths. Multi-level nesting is YAGNI right now.
pub trait PolicyDataResolver: Send + Sync {
    fn entity_exists(&self, entity: &str, conditions: &[(Vec<String>, serde_json::Value)]) -> bool;
}

// ---------------------------------------------------------------------------
// Per-scan `exists(...)` memoization
// ---------------------------------------------------------------------------

thread_local! {
    /// Active only inside an [`ExistsMemo`] scope. `None` means "don't
    /// memoize" — every other evaluation path (a single row read, a write
    /// check) hits the store directly, exactly as before.
    static EXISTS_MEMO: std::cell::RefCell<Option<HashMap<String, bool>>> =
        const { std::cell::RefCell::new(None) };
}

/// Memoize `exists(...)` results for the duration of one row scan.
///
/// Read policies are evaluated PER ROW, and a tenant gate is typically the
/// same question for every row of the scan — Revtrail's stat tables are gated
/// on
///
/// ```text
/// auth.tenantId == data.orgId && (
///   exists(StripeSubscription where referenceId == data.orgId and status == "active") || …
/// )
/// ```
///
/// which, over a full snapshot, asked the store the same three questions once
/// per row: thousands of identical subqueries per dashboard load, and the
/// dominant cost of the request.
///
/// Scoped rather than global on purpose. The cache lives on the stack of one
/// scan and is dropped with it, so a subscription that lapses is visible on
/// the very next request — a global or TTL cache would make an authorization
/// input stale for a window, which is not a trade worth making for a read
/// gate.
///
/// Keyed on the entity plus the fully-resolved condition values, so two rows
/// with different `orgId` do NOT share an answer. Correctness here is
/// load-bearing: a key that collapsed distinct tenants would leak rows.
pub struct ExistsMemo {
    _private: (),
}

impl ExistsMemo {
    /// Begin memoizing on this thread. Nested scopes are safe — the inner one
    /// reuses the outer's map and leaves teardown to the outermost.
    pub fn scope() -> Self {
        EXISTS_MEMO.with(|m| {
            let mut m = m.borrow_mut();
            if m.is_none() {
                *m = Some(HashMap::new());
            }
        });
        Self { _private: () }
    }
}

impl Drop for ExistsMemo {
    fn drop(&mut self) {
        EXISTS_MEMO.with(|m| {
            *m.borrow_mut() = None;
        });
    }
}

/// Cache key for one `exists(entity, conditions)` question.
///
/// Conditions arrive in AST order, which is stable for a given compiled
/// expression, so no sorting is needed — and sorting would be wrong anyway if
/// it ever collapsed two different orderings that the store treats
/// differently.
fn exists_memo_key(entity: &str, conditions: &[(Vec<String>, serde_json::Value)]) -> String {
    let mut key = String::with_capacity(entity.len() + 16 * conditions.len());
    key.push_str(entity);
    for (path, value) in conditions {
        key.push('\u{1f}');
        key.push_str(&path.join("."));
        key.push('=');
        // Serialized rather than Display'd so a string "1" and a number 1
        // can't collide into the same answer.
        key.push_str(&value.to_string());
    }
    key
}

/// `exists(...)` through the memo when one is active, straight to the store
/// otherwise.
fn resolve_exists(
    resolver: &dyn PolicyDataResolver,
    entity: &str,
    conditions: &[(Vec<String>, serde_json::Value)],
) -> bool {
    let key = EXISTS_MEMO.with(|m| {
        m.borrow()
            .as_ref()
            .map(|_| exists_memo_key(entity, conditions))
    });
    let Some(key) = key else {
        return resolver.entity_exists(entity, conditions);
    };
    if let Some(hit) = EXISTS_MEMO.with(|m| m.borrow().as_ref().and_then(|c| c.get(&key).copied()))
    {
        return hit;
    }
    // Resolved OUTSIDE the borrow: `entity_exists` reaches the row store,
    // which can re-enter policy evaluation, and holding the RefCell across
    // that would panic.
    let answer = resolver.entity_exists(entity, conditions);
    EXISTS_MEMO.with(|m| {
        if let Some(c) = m.borrow_mut().as_mut() {
            c.insert(key, answer);
        }
    });
    answer
}

/// Result of a policy check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyResult {
    Allowed,
    Denied { policy_name: String, reason: String },
}

/// Kind of entity access being checked. Drives which `allow_*` expression
/// the engine pulls from each manifest policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntityAction {
    Read,
    Insert,
    Update,
    Delete,
}

impl EntityAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Insert => "insert",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

impl PolicyResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, PolicyResult::Allowed)
    }
}

/// A policy engine that evaluates manifest policies against auth context.
///
/// Policy `allow` expressions are evaluated with simple pattern matching:
/// - `"auth.userId != null"` — requires authenticated user
/// - `"auth.userId == data.authorId"` — requires user matches data field
/// - `"auth.userId == input.authorId"` — requires user matches input field
/// - `"true"` — always allowed
///
/// This is NOT a full expression evaluator. It handles the common patterns
/// from the manifest contract. Complex expressions are treated as denied
/// with a clear message.
pub struct PolicyEngine {
    /// Entity policies bucketed by entity name, built once at `from_manifest`.
    /// Every per-row read check (list / query / snapshot pull / delta pull /
    /// broadcast fanout) needs "the policies for THIS entity" — a flat `Vec`
    /// forced an O(P) scan + a fresh `Vec` allocation on every single row. The
    /// map makes it an O(1) lookup returning a borrowed slice: no scan, no
    /// per-row allocation. (Mirrors the AST cache rationale on `compiled`.)
    entity_policies_by_name: HashMap<String, Vec<ManifestPolicy>>,
    action_policies: Vec<ManifestPolicy>,
    /// Pre-parsed AST for every policy expression, keyed by the raw
    /// expression string. Populated once at `from_manifest`, so per-row
    /// evaluation (list / query / search / the up-to-50k-row snapshot pull)
    /// is a map lookup instead of re-tokenizing + re-parsing the same
    /// constant expression on every single row. `Err` holds a ready-to-
    /// surface parse-error reason so a malformed expression still denies
    /// exactly as the uncached path did.
    compiled: HashMap<String, Result<Arc<Ast>, String>>,
    /// Entity name → its `sync_scope` predicate, for entities that declare
    /// one. Replication-only: see [`PolicyEngine::check_sync_scope`].
    sync_scopes: HashMap<String, String>,
    /// Set by the runtime after the row store comes up. None during
    /// boot + in unit tests that don't need `exists(...)` evaluation.
    /// `OnceLock` so set-once semantics are enforced (engine is shared
    /// via Arc across worker threads and a second `set_resolver` call
    /// would silently win the race).
    resolver: OnceLock<Arc<dyn PolicyDataResolver + Send + Sync>>,
}

impl PolicyEngine {
    /// Build a policy engine from a manifest.
    pub fn from_manifest(manifest: &AppManifest) -> Self {
        let mut entity_policies_by_name: HashMap<String, Vec<ManifestPolicy>> = HashMap::new();
        let mut action_policies = Vec::new();

        let mut compiled: HashMap<String, Result<Arc<Ast>, String>> = HashMap::new();
        for policy in &manifest.policies {
            if let Some(entity) = policy.entity.as_deref() {
                entity_policies_by_name
                    .entry(entity.to_string())
                    .or_default()
                    .push(policy.clone());
            }
            if policy.action.is_some() {
                action_policies.push(policy.clone());
            }
            // Pre-parse every expression this policy can present (the base
            // `allow` + all the `allow_*` overrides) so per-row evaluation
            // never re-parses.
            for expr in [
                policy.allow.as_str(),
                policy.allow_read.as_deref().unwrap_or(""),
                policy.allow_insert.as_deref().unwrap_or(""),
                policy.allow_update.as_deref().unwrap_or(""),
                policy.allow_delete.as_deref().unwrap_or(""),
                policy.allow_write.as_deref().unwrap_or(""),
            ] {
                if !expr.is_empty() && !compiled.contains_key(expr) {
                    compiled.insert(expr.to_string(), compile_expr(expr).map(Arc::new));
                }
            }
        }

        // Replication scopes ride the same compile cache as policies — they
        // are evaluated per row on the snapshot/bootstrap path, which is the
        // hottest per-row loop in the system.
        let mut sync_scopes: HashMap<String, String> = HashMap::new();
        for entity in &manifest.entities {
            let Some(expr) = entity.sync_scope.as_deref() else {
                continue;
            };
            let expr = expr.trim();
            if expr.is_empty() {
                continue;
            }
            if !compiled.contains_key(expr) {
                compiled.insert(expr.to_string(), compile_expr(expr).map(Arc::new));
            }
            sync_scopes.insert(entity.name.clone(), expr.to_string());
        }

        Self {
            entity_policies_by_name,
            action_policies,
            compiled,
            sync_scopes,
            resolver: OnceLock::new(),
        }
    }

    /// Is this row inside the caller's replication scope for `entity_name`?
    ///
    /// `Allowed` when the entity declares no scope — the default stays
    /// "replicate everything the caller may read". This is layered ON TOP of
    /// the read policy by the sync paths, never instead of it: a scope is a
    /// bandwidth decision, so narrowing one can only ever remove rows from a
    /// replica, never add them.
    ///
    /// A row with no data (a delete tombstone carrying nothing to test)
    /// evaluates the predicate against `None`, exactly as the read policy
    /// does, so the two agree on how they treat an absent row.
    pub fn check_sync_scope(
        &self,
        entity_name: &str,
        auth: &AuthContext,
        row: Option<&serde_json::Value>,
    ) -> PolicyResult {
        match self.sync_scopes.get(entity_name) {
            None => PolicyResult::Allowed,
            Some(expr) => self.eval_cached(expr, auth, row, None),
        }
    }

    /// Does `entity_name` declare a replication scope at all? Lets a caller
    /// skip per-row evaluation entirely on the common unscoped path.
    pub fn has_sync_scope(&self, entity_name: &str) -> bool {
        self.sync_scopes.contains_key(entity_name)
    }

    /// The policies registered for `entity_name` as a borrowed slice — O(1),
    /// no allocation. Empty slice when none are registered (the default-deny
    /// path keys off `is_empty()`).
    fn entity_policies_for(&self, entity_name: &str) -> &[ManifestPolicy] {
        self.entity_policies_by_name
            .get(entity_name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Evaluate a policy expression using the pre-parsed AST cache. Falls
    /// back to parsing on the fly only if the expression was never
    /// registered (shouldn't happen — `from_manifest` compiles them all).
    fn eval_cached(
        &self,
        expr: &str,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
        input: Option<&serde_json::Value>,
    ) -> PolicyResult {
        match self.compiled.get(expr) {
            Some(Ok(ast)) => evaluate_ast(ast, auth, data, input, self.resolver()),
            Some(Err(reason)) => PolicyResult::Denied {
                policy_name: String::new(),
                reason: reason.clone(),
            },
            None => evaluate_allow_inner(expr, auth, data, input, self.resolver()),
        }
    }

    /// Wire a `PolicyDataResolver` into the engine so `exists(...)`
    /// predicates can hit the row store. Call once after the store
    /// has been constructed at runtime boot. Subsequent calls are
    /// silently no-op'd (OnceLock semantics) so a re-init in a hot-
    /// reload scenario doesn't tear down the existing resolver.
    pub fn set_resolver(&self, resolver: Arc<dyn PolicyDataResolver + Send + Sync>) {
        let _ = self.resolver.set(resolver);
    }

    fn resolver(&self) -> Option<&dyn PolicyDataResolver> {
        self.resolver
            .get()
            .map(|a| a.as_ref() as &dyn PolicyDataResolver)
    }

    /// Which kind of entity access is being checked. Lets the engine pick
    /// the most specific `allow_*` expression from a manifest policy and
    /// fall back through the override chain when no specific rule is set.
    fn expr_for<'a>(policy: &'a ManifestPolicy, action: EntityAction) -> &'a str {
        // Resolution order (most specific first):
        //   read   → allow_read                      → allow
        //   insert → allow_insert → allow_write      → allow
        //   update → allow_update → allow_write      → allow
        //   delete → allow_delete → allow_write      → allow
        //
        // An empty string means "no expression provided" → falls through.
        let pick = |primary: &'a Option<String>, secondary: &'a Option<String>| -> &'a str {
            if let Some(s) = primary.as_deref() {
                if !s.is_empty() {
                    return s;
                }
            }
            if let Some(s) = secondary.as_deref() {
                if !s.is_empty() {
                    return s;
                }
            }
            policy.allow.as_str()
        };
        match action {
            EntityAction::Read => pick(&policy.allow_read, &None),
            EntityAction::Insert => pick(&policy.allow_insert, &policy.allow_write),
            EntityAction::Update => pick(&policy.allow_update, &policy.allow_write),
            EntityAction::Delete => pick(&policy.allow_delete, &policy.allow_write),
        }
    }

    fn check_entity(
        &self,
        entity_name: &str,
        action: EntityAction,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        // Admin bypasses all policies — EXCEPT a READ by an admin who has
        // selected a tenant. PYLON_ADMIN_TOKEN sessions, the user-row admin
        // field (`auth.user.adminField`), the PYLON_ADMIN_EMAILS allowlist,
        // and the Studio admin cookie all set is_admin so migrations /
        // Studio / ops scripts work without a wildcard policy.
        //
        // But an admin signed into the product UI with an org selected
        // (tenant_id set on the session) is operating *as* that tenant. If
        // the bypass also fired for their reads, every OTHER tenant's rows
        // would be served into their entity-API responses and streamed into
        // their browser sync replica — a cross-tenant data leak (an admin
        // of org A sees org B's recordings, invoices, viewers, …). The
        // row-level `auth.tenantId == data.orgId` predicate the app already
        // wrote is exactly the fence that should scope them, so for reads
        // with an active tenant we DON'T bypass — we evaluate the policy
        // like any tenant member. Admins with NO active tenant (operator
        // CLI, Studio's cross-tenant view) keep the full read bypass.
        //
        // Writes keep the unconditional bypass: they're tenant-stamped by
        // TenantScopePlugin and operator write tooling relies on it. The
        // cross-tenant *read* exposure is the boundary fixed here
        // (2026-06-04: admin-with-tenant sync/entity-API leak).
        let admin_read_scoped_to_tenant =
            matches!(action, EntityAction::Read) && auth.tenant_id().is_some();
        if auth.is_admin && !admin_read_scoped_to_tenant {
            return PolicyResult::Allowed;
        }

        let policies = self.entity_policies_for(entity_name);

        // Default-deny: an entity with NO policy is admin-only. The
        // alternative (default-allow) is the same Supabase RLS footgun
        // — devs ship with PII tables wide open because they forgot to
        // write a policy. Apps that genuinely want a public entity
        // declare `policy({entity: "X", allow: "true"})` explicitly.
        //
        // Pylon-internal scaffolding entities (anything starting with
        // an underscore — `_PylonSchemaVersion`, `_PylonJobs`,
        // `_PylonWorkflows`, etc.) bypass the gate. Apps don't write
        // policies for framework-owned tables, and the public API
        // doesn't expose them anyway.
        if policies.is_empty() {
            if entity_name.starts_with('_') {
                return PolicyResult::Allowed;
            }
            return PolicyResult::Denied {
                policy_name: "_default_deny".into(),
                reason: format!(
                    "No policy registered for entity \"{entity_name}\" — default-deny refuses access. Register a policy via policy({{entity: \"{entity_name}\", ...}}) or `policy({{entity: \"{entity_name}\", allow: \"true\"}})` for an intentionally-public entity."
                ),
            };
        }

        // Each policy on this entity acts as an additional gate. We
        // need at least ONE policy with an applicable allow expression
        // that evaluates true; an action with NO matching expression
        // anywhere defaults to deny (closes the second Supabase-style
        // hole where partial policies — `allowRead` only — silently
        // permitted writes).
        let mut any_rule_for_action = false;
        for policy in policies {
            let expr = Self::expr_for(policy, action);
            if expr.is_empty() {
                continue;
            }
            any_rule_for_action = true;
            match self.eval_cached(expr, auth, data, None) {
                PolicyResult::Denied { .. } => {
                    return PolicyResult::Denied {
                        policy_name: policy.name.clone(),
                        reason: format!(
                            "Policy \"{}\" denied ({}): {}",
                            policy.name,
                            action.as_str(),
                            expr
                        ),
                    };
                }
                PolicyResult::Allowed => {}
            }
        }

        if !any_rule_for_action {
            return PolicyResult::Denied {
                policy_name: "_default_deny".into(),
                reason: format!(
                    "Entity \"{entity_name}\" has policies but no rule covering action \"{}\" — default-deny refuses access. Add allow{} (or a wildcard `allow`) to one of the entity's policies.",
                    action.as_str(),
                    match action {
                        EntityAction::Read => "Read",
                        EntityAction::Insert => "Insert",
                        EntityAction::Update => "Update",
                        EntityAction::Delete => "Delete",
                    }
                ),
            };
        }

        PolicyResult::Allowed
    }

    /// Check if an entity read is allowed for the given auth context.
    /// `data` is the row being accessed (for field-level checks).
    pub fn check_entity_read(
        &self,
        entity_name: &str,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        self.check_entity(entity_name, EntityAction::Read, auth, data)
    }

    /// Pre-check for bulk scans (cursor pagination, filtered queries)
    /// where the handler will apply per-row filtering downstream and
    /// only needs to know whether the policy hard-denies the whole
    /// entity for this caller.
    ///
    /// Semantics:
    ///   - Hard-deny (`allow: "false"`, default-deny for missing rule)
    ///     → Denied. Handler returns 403; row filtering is skipped.
    ///   - Data-independent expressions (e.g. `auth.userId != null`)
    ///     → evaluated normally with `data: None` — if false, Denied.
    ///   - Data-dependent expressions (anything referencing `data.X`)
    ///     → Allowed. Handler scans the entity and per-row filtering
    ///     drops rows the caller can't read. Without this special
    ///     case, the data-less evaluation would treat `data.X` as
    ///     Null, `auth.Y == Null` resolves to false, and EVERY tenant-
    ///     scoped read policy hard-denied the whole cursor — every
    ///     dashboard using `db.useQuery("TenantThing")` got 403 +
    ///     empty replica. The per-row filter inside the cursor
    ///     handler is the real security boundary; this pre-check
    ///     just lets us short-circuit when no policy could possibly
    ///     permit a row regardless of its contents.
    ///
    /// Admin bypasses the scan pre-check — EXCEPT an admin with an active
    /// tenant, who is scoped to that tenant on bulk reads (same boundary as
    /// `check_entity_read`). When scoped, a data-dependent tenant predicate
    /// (`auth.tenantId == data.orgId`) still defers to per-row filtering
    /// below, so the entity is scanned and the per-row read fence drops the
    /// foreign rows. Admins with no active tenant (operator / Studio) keep
    /// the bypass.
    pub fn check_entity_scan(&self, entity_name: &str, auth: &AuthContext) -> PolicyResult {
        if auth.is_admin && auth.tenant_id().is_none() {
            return PolicyResult::Allowed;
        }

        let policies = self.entity_policies_for(entity_name);

        // Default-deny / framework-internal handling mirrors
        // check_entity (kept in sync with that method's branch).
        if policies.is_empty() {
            if entity_name.starts_with('_') {
                return PolicyResult::Allowed;
            }
            return PolicyResult::Denied {
                policy_name: "_default_deny".into(),
                reason: format!(
                    "No policy registered for entity \"{entity_name}\" — default-deny refuses access. Register a policy via policy({{entity: \"{entity_name}\", ...}}) or `policy({{entity: \"{entity_name}\", allow: \"true\"}})` for an intentionally-public entity."
                ),
            };
        }

        let mut any_rule_for_action = false;
        for policy in policies {
            let expr = Self::expr_for(policy, EntityAction::Read);
            if expr.is_empty() {
                continue;
            }
            any_rule_for_action = true;
            // Skip per-row predicates at scan-time. If the rule references a
            // row field (`data.` or its `existing.` alias), it's filterable —
            // the caller runs the full check per row. Otherwise evaluate now
            // and surface the result, so `allow: "false"` and missing-auth
            // gates (`auth.userId != null`) still 403 early. (A row-less `now`
            // gate has neither prefix and is correctly evaluated here.)
            if expr.contains("data.") || expr.contains("existing.") {
                continue;
            }
            match self.eval_cached(expr, auth, None, None) {
                PolicyResult::Denied { .. } => {
                    return PolicyResult::Denied {
                        policy_name: policy.name.clone(),
                        reason: format!("Policy \"{}\" denied (read scan): {}", policy.name, expr),
                    };
                }
                PolicyResult::Allowed => {}
            }
        }

        if !any_rule_for_action {
            return PolicyResult::Denied {
                policy_name: "_default_deny".into(),
                reason: format!(
                    "Entity \"{entity_name}\" has policies but no rule covering action \"read\" — default-deny refuses access. Add allowRead (or a wildcard `allow`) to one of the entity's policies."
                ),
            };
        }

        PolicyResult::Allowed
    }

    /// True if the entity's READ policy can NEVER permit any row for ANY
    /// caller — an explicit deny (`allowRead: "false"` on every read rule).
    ///
    /// Unlike [`Self::check_entity_scan`], this does NOT bypass for admin.
    /// It answers a narrower question — "should this entity ever be BULK-
    /// replicated into a client snapshot?" — and the answer for an
    /// explicitly deny-all table is NO, even for an admin. Admins still read
    /// such a table via `/api/entities` or a server function (where the
    /// admin bypass applies); they just don't get the whole table streamed
    /// into their browser replica on every connect. This is what keeps a
    /// large append-only deny-all table (e.g. an audit log) out of the sync
    /// snapshot regardless of caller — without it, an admin session pulls
    /// the entire ever-growing table every snapshot, which (with the
    /// no-id-ceiling pager) never converges → a since=0 resync storm.
    pub fn is_read_statically_denied(&self, entity_name: &str) -> bool {
        let read_exprs: Vec<&str> = self
            .entity_policies_for(entity_name)
            .iter()
            .map(|p| Self::expr_for(p, EntityAction::Read))
            .filter(|e| !e.is_empty())
            .collect();
        // No read rule at all → default-deny, which is per-caller and admin
        // bypasses; that's not a deliberate "never sync" marker, so don't
        // exclude it here.
        if read_exprs.is_empty() {
            return false;
        }
        // Every read rule is a literal `false` → no caller, admin included,
        // can ever read a row, so it must never be bulk-snapshotted.
        read_exprs.iter().all(|e| e.trim() == "false")
    }

    /// Check if an entity write (insert/update/delete) is allowed.
    ///
    /// `data` is the incoming payload (for insert/update) or the existing row
    /// (for delete). Delegates to the specific insert/update/delete path
    /// when the caller knows the operation; kept as a generic entry point
    /// for legacy call sites that don't discriminate.
    pub fn check_entity_write(
        &self,
        entity_name: &str,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        self.check_entity(entity_name, EntityAction::Insert, auth, data)
    }

    /// Check if an entity insert is allowed. `data` is the incoming row.
    pub fn check_entity_insert(
        &self,
        entity_name: &str,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        self.check_entity(entity_name, EntityAction::Insert, auth, data)
    }

    /// Check if an entity update is allowed. `data` should be the existing
    /// row so ownership checks like `data.authorId == auth.userId` evaluate
    /// against truth instead of the incoming patch.
    pub fn check_entity_update(
        &self,
        entity_name: &str,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        self.check_entity(entity_name, EntityAction::Update, auth, data)
    }

    /// Check if an entity delete is allowed. `data` is the row about to be
    /// removed so delete-gates can look at the row's author/tenant fields.
    pub fn check_entity_delete(
        &self,
        entity_name: &str,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        self.check_entity(entity_name, EntityAction::Delete, auth, data)
    }

    /// Check if an action execution is allowed.
    /// `input` is the action input data.
    pub fn check_action(
        &self,
        action_name: &str,
        auth: &AuthContext,
        input: Option<&serde_json::Value>,
    ) -> PolicyResult {
        if auth.is_admin {
            return PolicyResult::Allowed;
        }

        let policies: Vec<&ManifestPolicy> = self
            .action_policies
            .iter()
            .filter(|p| p.action.as_deref() == Some(action_name))
            .collect();

        if policies.is_empty() {
            return PolicyResult::Allowed;
        }

        for policy in &policies {
            match self.eval_cached(&policy.allow, auth, None, input) {
                PolicyResult::Denied { .. } => {
                    return PolicyResult::Denied {
                        policy_name: policy.name.clone(),
                        reason: format!("Policy \"{}\" denied: {}", policy.name, policy.allow),
                    };
                }
                PolicyResult::Allowed => {}
            }
        }

        PolicyResult::Allowed
    }
}

/// Parse a comma-separated list of quoted strings (single or double quotes).
///
/// `"a", 'b,c', "d"` → `["a", "b,c", "d"]`. Respects quote boundaries so a
/// comma inside a quoted string is treated as part of the string, not a
/// separator. Used for `hasAnyRole` so role names containing commas aren't
/// silently split. Returns an error on unterminated strings or unquoted
/// tokens.
#[cfg(test)]
fn parse_quoted_string_list(s: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace and commas between items.
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        if quote != b'"' && quote != b'\'' {
            return Err(format!(
                "expected quoted string at byte {i}, got {:?}",
                quote as char
            ));
        }
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        if i >= bytes.len() {
            return Err("unterminated quoted string".into());
        }
        let piece = &s[start..i];
        out.push(piece.to_string());
        i += 1; // skip closing quote
    }
    Ok(out)
}

/// Evaluate an `allow` expression against auth context and data.
///
/// Supports the following grammar (informal):
/// ```text
///   expr    := or
///   or      := and ("||" and)*
///   and     := not ("&&" not)*
///   not     := "!" not | primary
///   primary := "true" | "false"
///            | "(" expr ")"
///            | call
///            | path (("==" | "!=") atom)?
///   atom    := "null" | "true" | "false" | string | path
///   call    := "auth.hasRole" "(" string ")"
///            | "auth.hasAnyRole" "(" string ("," string)* ")"
///   path    := IDENT ("." IDENT)*    // auth.userId, data.author.id, etc.
/// ```
/// Existing primitives (`auth.userId != null`, `auth.isAdmin`,
/// `auth.hasRole(...)`, `auth.hasAnyRole(...)`, `auth.userId == data.<path>`)
/// are special cases of the grammar; old schemas keep working unchanged.
/// Back-compat wrapper used by call sites that don't have a resolver.
/// New code should call `evaluate_allow_inner` directly.
#[cfg(test)]
fn evaluate_allow(
    expr: &str,
    auth: &AuthContext,
    data: Option<&serde_json::Value>,
    input: Option<&serde_json::Value>,
) -> PolicyResult {
    evaluate_allow_inner(expr, auth, data, input, None)
}

/// Evaluate a single policy expression against an explicit context —
/// the dry-run entry point behind `pylon policy test`. Same tokenizer,
/// parser, and evaluator as production enforcement, so an expression
/// that passes here behaves identically inside a deployed policy (the
/// one exception: the `existing(...)` resolver isn't wired, matching
/// how data-independent checks run). Parse errors surface as Denied
/// with the parser's reason, exactly as enforcement treats them.
pub fn evaluate_expression(
    expr: &str,
    auth: &AuthContext,
    data: Option<&serde_json::Value>,
    input: Option<&serde_json::Value>,
) -> PolicyResult {
    evaluate_allow_inner(expr, auth, data, input, None)
}

/// Tokenize + parse + at-end check an expression into an `Ast`. `Err` is a
/// ready-to-surface "Policy parse error" / "Trailing tokens" reason. Split
/// out from evaluation so the parsed AST can be cached once (see
/// `PolicyEngine.compiled`) rather than re-parsed on every row.
fn compile_expr(expr: &str) -> Result<Ast, String> {
    let tokens = tokenize(expr).map_err(|e| format!("Policy parse error: {e} (in {expr:?})"))?;
    let mut parser = Parser::new(&tokens);
    let ast = parser
        .parse_expr()
        .map_err(|e| format!("Policy parse error: {e} (in {expr:?})"))?;
    if !parser.at_end() {
        return Err(format!("Trailing tokens in expression: {expr:?}"));
    }
    Ok(ast)
}

/// Evaluate an already-parsed `Ast` against the auth/data/input context.
fn evaluate_ast(
    ast: &Ast,
    auth: &AuthContext,
    data: Option<&serde_json::Value>,
    input: Option<&serde_json::Value>,
    resolver: Option<&dyn PolicyDataResolver>,
) -> PolicyResult {
    let env = EvalEnv {
        auth,
        data,
        input,
        resolver,
        now: pylon_kernel::util::now_iso(),
    };
    match env.eval(ast) {
        EvalResult::True => PolicyResult::Allowed,
        EvalResult::False(reason) => PolicyResult::Denied {
            policy_name: String::new(),
            reason,
        },
    }
}

fn evaluate_allow_inner(
    expr: &str,
    auth: &AuthContext,
    data: Option<&serde_json::Value>,
    input: Option<&serde_json::Value>,
    resolver: Option<&dyn PolicyDataResolver>,
) -> PolicyResult {
    match compile_expr(expr) {
        Ok(ast) => evaluate_ast(&ast, auth, data, input, resolver),
        Err(reason) => PolicyResult::Denied {
            policy_name: String::new(),
            reason,
        },
    }
}

// ---------------------------------------------------------------------------
// Expression parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    True,
    False,
    Null,
    And, // &&
    Or,  // ||
    Not, // !
    Eq,  // ==
    Neq, // !=
    Lt,  // <
    Lte, // <=
    Gt,  // >
    Gte, // >=
    LParen,
    RParen,
    Comma,
    Ident(String),
    Str(String),
    /// Numeric literal lexeme (parsed to f64 when built into the AST).
    Num(String),
}

fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' | b'\n' | b'\r' => {
                i += 1;
            }
            b'(' => {
                out.push(Token::LParen);
                i += 1;
            }
            b')' => {
                out.push(Token::RParen);
                i += 1;
            }
            b',' => {
                out.push(Token::Comma);
                i += 1;
            }
            b'&' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'&' {
                    out.push(Token::And);
                    i += 2;
                } else {
                    return Err("single `&` — did you mean `&&`?".into());
                }
            }
            b'|' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                    out.push(Token::Or);
                    i += 2;
                } else {
                    return Err("single `|` — did you mean `||`?".into());
                }
            }
            b'=' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    out.push(Token::Eq);
                    i += 2;
                } else {
                    return Err("single `=` — did you mean `==`?".into());
                }
            }
            b'!' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    out.push(Token::Neq);
                    i += 2;
                } else {
                    out.push(Token::Not);
                    i += 1;
                }
            }
            b'<' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    out.push(Token::Lte);
                    i += 2;
                } else {
                    out.push(Token::Lt);
                    i += 1;
                }
            }
            b'>' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    out.push(Token::Gte);
                    i += 2;
                } else {
                    out.push(Token::Gt);
                    i += 1;
                }
            }
            // Numeric literal: `[-]?digits[.digits]?`. A leading `-` is only
            // a number when a digit follows (there is no subtraction operator,
            // so a bare `-` stays an error). The lexeme is parsed to f64 at the
            // AST stage; storing it as a string keeps `Token` Eq-able.
            c if c.is_ascii_digit()
                || (c == b'-' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()) =>
            {
                let start = i;
                if c == b'-' {
                    i += 1;
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                out.push(Token::Num(src[start..i].to_string()));
            }
            b'"' | b'\'' => {
                // Parse the literal as chars (not bytes) so multi-byte UTF-8
                // round-trips intact. Only a fixed escape set is honored;
                // unknown escapes error rather than silently dropping the
                // backslash.
                let quote = c as char;
                // Skip opening quote, then walk the rest of the string as
                // a char iterator. Build `unescaped` directly since escapes
                // are resolved inline.
                let rest = &src[i + 1..];
                let mut chars = rest.char_indices();
                let mut unescaped = String::new();
                let mut closed_at: Option<usize> = None;
                while let Some((rel, ch)) = chars.next() {
                    if ch == quote {
                        closed_at = Some(i + 1 + rel + ch.len_utf8());
                        break;
                    }
                    if ch == '\\' {
                        let (_rel2, esc) = chars
                            .next()
                            .ok_or_else(|| "unterminated string literal".to_string())?;
                        match esc {
                            '\\' => unescaped.push('\\'),
                            '"' => unescaped.push('"'),
                            '\'' => unescaped.push('\''),
                            'n' => unescaped.push('\n'),
                            'r' => unescaped.push('\r'),
                            't' => unescaped.push('\t'),
                            '0' => unescaped.push('\0'),
                            other => {
                                return Err(format!("unknown string escape `\\{other}`"));
                            }
                        }
                    } else {
                        unescaped.push(ch);
                    }
                }
                let close = closed_at.ok_or_else(|| "unterminated string literal".to_string())?;
                out.push(Token::Str(unescaped));
                i = close;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < bytes.len() {
                    let ch = bytes[i];
                    if ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'.' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let word = &src[start..i];
                match word {
                    "true" => out.push(Token::True),
                    "false" => out.push(Token::False),
                    "null" => out.push(Token::Null),
                    _ => out.push(Token::Ident(word.to_string())),
                }
            }
            other => {
                return Err(format!("unexpected character {:?}", other as char));
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
enum Ast {
    True,
    False,
    Not(Box<Ast>),
    And(Box<Ast>, Box<Ast>),
    Or(Box<Ast>, Box<Ast>),
    Eq(Box<Ast>, Box<Ast>),
    Neq(Box<Ast>, Box<Ast>),
    /// Ordering comparisons (`<`, `<=`, `>`, `>=`). Numeric when either side
    /// resolves to a number; chronological when both are RFC-3339 timestamps;
    /// lexicographic for other string pairs; false (deny-safe) for anything
    /// not orderable (null, mixed, booleans).
    Lt(Box<Ast>, Box<Ast>),
    Lte(Box<Ast>, Box<Ast>),
    Gt(Box<Ast>, Box<Ast>),
    Gte(Box<Ast>, Box<Ast>),
    /// `auth.hasRole("x")`
    HasRole(String),
    /// `auth.hasAnyRole("a", "b", ...)`
    HasAnyRole(Vec<String>),
    /// `exists(Entity where field == path/literal [and field == path/literal]*)`
    ///
    /// Each condition compares a field on rows of `entity` against an
    /// expression resolved in the current `EvalEnv` (so `auth.userId`,
    /// `data.orgId`, etc. all work as RHS). True iff the policy data
    /// resolver finds at least one matching row.
    Exists {
        entity: String,
        conditions: Vec<(Vec<String>, Ast)>,
    },
    /// A path like `auth.userId` or `data.author.id`.
    Path(Vec<String>),
    /// A string literal.
    Str(String),
    /// A numeric literal.
    Num(f64),
    /// `null` literal.
    Null,
    /// Degenerate: bare `auth.isAdmin` etc. resolves to a boolean.
    Bool(bool),
}

/// Cap recursive descent so a pathological input like `((((...!x))))` can't
/// stack-overflow the server thread. 64 is far beyond any realistic policy —
/// a hand-authored expression rarely nests more than 3–4 levels.
const MAX_PARSE_DEPTH: usize = 64;

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            pos: 0,
            depth: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<&Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// Enter one level of recursion, erroring if we exceed the cap.
    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            return Err(format!(
                "policy expression nested deeper than {MAX_PARSE_DEPTH} levels"
            ));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn parse_expr(&mut self) -> Result<Ast, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Ast, String> {
        self.enter()?;
        let mut lhs = self.parse_and()?;
        while matches!(self.peek(), Some(Token::Or)) {
            self.bump();
            let rhs = self.parse_and()?;
            lhs = Ast::Or(Box::new(lhs), Box::new(rhs));
        }
        self.leave();
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Ast, String> {
        let mut lhs = self.parse_comparison()?;
        while matches!(self.peek(), Some(Token::And)) {
            self.bump();
            let rhs = self.parse_comparison()?;
            lhs = Ast::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// Comparison binds LOOSER than `!`, so `!x == null` parses as
    /// `(!x) == null` — matching conventional precedence in languages like
    /// JS/Rust. Previously `parse_primary` ate `== null` greedily, causing
    /// `!x == null` to evaluate as `!(x == null)` which is almost never
    /// what a rule author intends.
    fn parse_comparison(&mut self) -> Result<Ast, String> {
        let lhs = self.parse_not()?;
        match self.peek() {
            Some(Token::Eq) => {
                self.bump();
                let rhs = self.parse_atom()?;
                Ok(Ast::Eq(Box::new(lhs), Box::new(rhs)))
            }
            Some(Token::Neq) => {
                self.bump();
                let rhs = self.parse_atom()?;
                Ok(Ast::Neq(Box::new(lhs), Box::new(rhs)))
            }
            Some(Token::Lt) => {
                self.bump();
                let rhs = self.parse_atom()?;
                Ok(Ast::Lt(Box::new(lhs), Box::new(rhs)))
            }
            Some(Token::Lte) => {
                self.bump();
                let rhs = self.parse_atom()?;
                Ok(Ast::Lte(Box::new(lhs), Box::new(rhs)))
            }
            Some(Token::Gt) => {
                self.bump();
                let rhs = self.parse_atom()?;
                Ok(Ast::Gt(Box::new(lhs), Box::new(rhs)))
            }
            Some(Token::Gte) => {
                self.bump();
                let rhs = self.parse_atom()?;
                Ok(Ast::Gte(Box::new(lhs), Box::new(rhs)))
            }
            _ => Ok(lhs),
        }
    }

    fn parse_not(&mut self) -> Result<Ast, String> {
        if matches!(self.peek(), Some(Token::Not)) {
            self.bump();
            self.enter()?;
            let inner = self.parse_not()?;
            self.leave();
            return Ok(Ast::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Ast, String> {
        match self.peek().cloned() {
            Some(Token::True) => {
                self.bump();
                Ok(Ast::True)
            }
            Some(Token::False) => {
                self.bump();
                Ok(Ast::False)
            }
            Some(Token::Null) => {
                self.bump();
                Ok(Ast::Null)
            }
            Some(Token::Str(s)) => {
                self.bump();
                Ok(Ast::Str(s))
            }
            Some(Token::Num(s)) => {
                self.bump();
                let n = s
                    .parse::<f64>()
                    .map_err(|_| format!("invalid numeric literal {s:?}"))?;
                Ok(Ast::Num(n))
            }
            Some(Token::LParen) => {
                self.bump();
                self.enter()?;
                let inner = self.parse_expr()?;
                self.leave();
                match self.peek() {
                    Some(Token::RParen) => {
                        self.bump();
                    }
                    _ => return Err("expected `)`".into()),
                }
                Ok(inner)
            }
            Some(Token::Ident(name)) => {
                self.bump();
                // Three cases: bare path, builtin function call
                // (`auth.hasRole(...)`), or the special `exists(...)`
                // form with its own internal grammar.
                if matches!(self.peek(), Some(Token::LParen)) {
                    self.bump();
                    if name == "exists" {
                        return self.parse_exists_body();
                    }
                    let args = self.parse_string_args()?;
                    match self.peek() {
                        Some(Token::RParen) => {
                            self.bump();
                        }
                        _ => return Err("expected `)` after function args".into()),
                    }
                    return self.build_call(&name, args);
                }
                // Comparison (==, !=) is handled by parse_comparison above —
                // intentionally NOT consumed here, so `!x == null` parses as
                // `(!x) == null` instead of `!(x == null)`.
                Ok(Ast::Path(split_path(&name)))
            }
            Some(other) => Err(format!("unexpected token {other:?}")),
            None => Err("unexpected end of expression".into()),
        }
    }

    fn parse_string_args(&mut self) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        loop {
            match self.peek().cloned() {
                Some(Token::Str(s)) => {
                    self.bump();
                    out.push(s);
                }
                Some(Token::RParen) => break,
                Some(other) => {
                    return Err(format!("expected quoted string argument, got {other:?}"));
                }
                None => return Err("unexpected end inside function args".into()),
            }
            match self.peek() {
                Some(Token::Comma) => {
                    self.bump();
                }
                Some(Token::RParen) => break,
                _ => break,
            }
        }
        Ok(out)
    }

    fn build_call(&mut self, name: &str, args: Vec<String>) -> Result<Ast, String> {
        match name {
            "auth.hasRole" => {
                if args.len() != 1 {
                    return Err("auth.hasRole takes exactly one string argument".into());
                }
                Ok(Ast::HasRole(args.into_iter().next().unwrap()))
            }
            "auth.hasAnyRole" => {
                if args.is_empty() {
                    return Err("auth.hasAnyRole takes at least one argument".into());
                }
                Ok(Ast::HasAnyRole(args))
            }
            other => Err(format!("unknown function \"{other}(...)\"")),
        }
    }

    /// Parse the inside of `exists(<entity> where <cond> [and <cond>]* )`.
    /// Caller has already consumed the `exists` ident and the opening
    /// `(`. We finish at the matching `)`.
    ///
    /// Grammar is intentionally narrower than the outer expression: only
    /// `==` comparisons, only `and` between them (no `or`, no `!`, no
    /// nested `exists`). That keeps the storage-side cost predictable
    /// (one indexed lookup with all clauses ANDed) and avoids the
    /// recursion-pit a full sub-expression would open up. Wider grammars
    /// can grow in once we see a real use case.
    fn parse_exists_body(&mut self) -> Result<Ast, String> {
        // <entity>
        let entity = match self.peek().cloned() {
            Some(Token::Ident(name)) => {
                self.bump();
                name
            }
            other => {
                return Err(format!("exists(): expected entity name, got {other:?}"));
            }
        };

        // `where`
        match self.peek().cloned() {
            Some(Token::Ident(kw)) if kw == "where" => {
                self.bump();
            }
            other => {
                return Err(format!(
                    "exists({entity} ...): expected `where`, got {other:?}"
                ));
            }
        }

        // One or more `<path> == <expr>` clauses joined by `and`.
        let mut conditions: Vec<(Vec<String>, Ast)> = Vec::new();
        loop {
            let path = match self.peek().cloned() {
                Some(Token::Ident(name)) => {
                    self.bump();
                    split_path(&name)
                }
                other => {
                    return Err(format!(
                        "exists({entity} where ...): expected field path, got {other:?}"
                    ));
                }
            };
            match self.peek() {
                Some(Token::Eq) => {
                    self.bump();
                }
                other => {
                    return Err(format!(
                        "exists({entity} where {path:?} ...): expected `==`, got {other:?}"
                    ));
                }
            }
            // RHS uses the full atom grammar — strings, nulls, bare
            // paths into auth/data/input. We DON'T allow nested
            // `exists` here: a nested exists() would fire one
            // additional DB query per evaluation, and the codex
            // Wave-3 review (Wave-3 #2) found that the MAX_PARSE_DEPTH
            // cap of 64 multiplied by per-row evaluation in
            // check_entity_scan could land a malicious schema's
            // policy doing 64 * <page_size> DB roundtrips per scan.
            // Explicit reject here closes that amplifier.
            self.enter()?;
            let rhs = self.parse_or()?;
            self.leave();
            if ast_contains_exists(&rhs) {
                return Err(format!(
                    "exists({entity} where ...): nested exists() in RHS is not supported"
                ));
            }
            conditions.push((path, rhs));

            match self.peek().cloned() {
                Some(Token::And) => {
                    self.bump();
                    // Continue to next condition.
                }
                Some(Token::RParen) => {
                    self.bump();
                    break;
                }
                Some(Token::Ident(kw)) if kw == "and" => {
                    // Bare "and" ident — accept for ergonomic parity
                    // with `&&` (Yapless app.ts uses this form).
                    self.bump();
                }
                other => {
                    return Err(format!(
                        "exists({entity} where ...): expected `and` or `)`, got {other:?}"
                    ));
                }
            }
        }

        if conditions.is_empty() {
            return Err(format!(
                "exists({entity} where ...): at least one condition required"
            ));
        }

        Ok(Ast::Exists { entity, conditions })
    }

    fn parse_atom(&mut self) -> Result<Ast, String> {
        match self.peek().cloned() {
            Some(Token::Null) => {
                self.bump();
                Ok(Ast::Null)
            }
            Some(Token::True) => {
                self.bump();
                Ok(Ast::Bool(true))
            }
            Some(Token::False) => {
                self.bump();
                Ok(Ast::Bool(false))
            }
            Some(Token::Str(s)) => {
                self.bump();
                Ok(Ast::Str(s))
            }
            Some(Token::Num(s)) => {
                self.bump();
                let n = s
                    .parse::<f64>()
                    .map_err(|_| format!("invalid numeric literal {s:?}"))?;
                Ok(Ast::Num(n))
            }
            Some(Token::Ident(name)) => {
                self.bump();
                Ok(Ast::Path(split_path(&name)))
            }
            Some(other) => Err(format!("expected atom, got {other:?}")),
            None => Err("unexpected end of expression in atom".into()),
        }
    }
}

/// Walk an Ast and return true iff any node is `Ast::Exists`. Used
/// by `parse_exists_body` to reject nested `exists(...)` in the
/// RHS of a condition — otherwise a single policy could fan out to
/// MAX_PARSE_DEPTH × page_size DB queries (codex Wave-3 #2).
fn ast_contains_exists(ast: &Ast) -> bool {
    match ast {
        Ast::Exists { .. } => true,
        Ast::Not(inner) => ast_contains_exists(inner),
        Ast::And(l, r)
        | Ast::Or(l, r)
        | Ast::Eq(l, r)
        | Ast::Neq(l, r)
        | Ast::Lt(l, r)
        | Ast::Lte(l, r)
        | Ast::Gt(l, r)
        | Ast::Gte(l, r) => ast_contains_exists(l) || ast_contains_exists(r),
        Ast::True
        | Ast::False
        | Ast::HasRole(_)
        | Ast::HasAnyRole(_)
        | Ast::Path(_)
        | Ast::Str(_)
        | Ast::Num(_)
        | Ast::Null
        | Ast::Bool(_) => false,
    }
}

fn split_path(s: &str) -> Vec<String> {
    s.split('.').map(|p| p.to_string()).collect()
}

struct EvalEnv<'a> {
    auth: &'a AuthContext,
    data: Option<&'a serde_json::Value>,
    input: Option<&'a serde_json::Value>,
    /// Set when the policy engine has been given a resolver so
    /// `exists(...)` can hit the row store. None for callers that
    /// don't supply one (most unit tests, scan-mode pre-checks);
    /// `Ast::Exists` then evaluates to Denied with a clear reason.
    resolver: Option<&'a dyn PolicyDataResolver>,
    /// Current UTC time (ISO-8601), bound to the `now` identifier so policies
    /// can express time windows (`data.publishAt <= now`). Computed once per
    /// evaluation so every `now` in one expression sees the same instant.
    now: String,
}

#[derive(Debug)]
enum EvalResult {
    True,
    False(String),
}

#[derive(Debug, Clone)]
enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
    Null,
}

impl<'a> EvalEnv<'a> {
    fn eval(&self, ast: &Ast) -> EvalResult {
        match ast {
            Ast::True => EvalResult::True,
            Ast::False => EvalResult::False("Expression is false".into()),
            Ast::Not(inner) => match self.eval(inner) {
                EvalResult::True => EvalResult::False("Negated expression was true".into()),
                EvalResult::False(_) => EvalResult::True,
            },
            Ast::And(l, r) => match self.eval(l) {
                EvalResult::False(reason) => EvalResult::False(reason),
                EvalResult::True => self.eval(r),
            },
            Ast::Or(l, r) => match self.eval(l) {
                EvalResult::True => EvalResult::True,
                EvalResult::False(reason_l) => match self.eval(r) {
                    EvalResult::True => EvalResult::True,
                    EvalResult::False(reason_r) => {
                        EvalResult::False(format!("{reason_l}; and {reason_r}"))
                    }
                },
            },
            Ast::Eq(l, r) => {
                let lv = self.value_of(l);
                let rv = self.value_of(r);
                if values_eq(&lv, &rv) {
                    EvalResult::True
                } else {
                    EvalResult::False(format!("{lv:?} != {rv:?}"))
                }
            }
            Ast::Neq(l, r) => {
                let lv = self.value_of(l);
                let rv = self.value_of(r);
                if values_eq(&lv, &rv) {
                    EvalResult::False(format!("{lv:?} == {rv:?}"))
                } else {
                    EvalResult::True
                }
            }
            Ast::Lt(l, r) | Ast::Lte(l, r) | Ast::Gt(l, r) | Ast::Gte(l, r) => {
                let lv = self.value_of(l);
                let rv = self.value_of(r);
                // Not-orderable operands (null, mixed types, booleans) yield
                // `None` → the comparison is false. This is deny-safe: an
                // unresolvable `data.publishAt <= now` denies rather than
                // allows.
                let ord = compare_values(&lv, &rv);
                let pass = match ast {
                    Ast::Lt(..) => ord == Some(Ordering::Less),
                    Ast::Lte(..) => matches!(ord, Some(Ordering::Less | Ordering::Equal)),
                    Ast::Gt(..) => ord == Some(Ordering::Greater),
                    Ast::Gte(..) => matches!(ord, Some(Ordering::Greater | Ordering::Equal)),
                    _ => unreachable!(),
                };
                if pass {
                    EvalResult::True
                } else {
                    EvalResult::False(format!("comparison {lv:?} vs {rv:?} did not hold"))
                }
            }
            Ast::HasRole(role) => {
                if self.auth.has_role(role) {
                    EvalResult::True
                } else {
                    EvalResult::False(format!("Missing required role \"{role}\""))
                }
            }
            Ast::HasAnyRole(roles) => {
                let refs: Vec<&str> = roles.iter().map(|s| s.as_str()).collect();
                if self.auth.has_any_role(&refs) {
                    EvalResult::True
                } else {
                    EvalResult::False(format!("Missing any of required roles: {refs:?}"))
                }
            }
            Ast::Exists { entity, conditions } => {
                let resolver = match self.resolver {
                    Some(r) => r,
                    None => {
                        return EvalResult::False(format!(
                            "exists({entity} ...) not evaluable: no data resolver wired \
                             (this evaluation path doesn't have row-store access — \
                             call check_entity_*_with_resolver from the route handler)"
                        ));
                    }
                };
                // Resolve each RHS in the current env, package as
                // (path, json_value) pairs, hand to the resolver.
                let mut packed: Vec<(Vec<String>, serde_json::Value)> =
                    Vec::with_capacity(conditions.len());
                for (path, rhs_ast) in conditions {
                    let v = self.value_of(rhs_ast);
                    let json = match v {
                        Value::Str(s) => serde_json::Value::String(s),
                        Value::Num(n) => serde_json::Number::from_f64(n)
                            .map(serde_json::Value::Number)
                            .unwrap_or(serde_json::Value::Null),
                        Value::Bool(b) => serde_json::Value::Bool(b),
                        Value::Null => serde_json::Value::Null,
                    };
                    packed.push((path.clone(), json));
                }
                if resolve_exists(resolver, entity, &packed) {
                    EvalResult::True
                } else {
                    EvalResult::False(format!("exists({entity} ...) matched no rows"))
                }
            }
            Ast::Path(_) | Ast::Str(_) | Ast::Num(_) | Ast::Null | Ast::Bool(_) => {
                // Bare value as boolean expression.
                match self.value_of(ast) {
                    Value::Bool(true) => EvalResult::True,
                    Value::Bool(false) => EvalResult::False("Expression evaluated to false".into()),
                    Value::Null => EvalResult::False("Expression evaluated to null".into()),
                    Value::Num(n) => {
                        // Nonzero is truthy (matches JS-ish intuition).
                        if n == 0.0 {
                            EvalResult::False("Zero".into())
                        } else {
                            EvalResult::True
                        }
                    }
                    Value::Str(s) => {
                        // Non-empty string is truthy (matches JS-ish intuition).
                        if s.is_empty() {
                            EvalResult::False("Empty string".into())
                        } else {
                            EvalResult::True
                        }
                    }
                }
            }
        }
    }

    fn value_of(&self, ast: &Ast) -> Value {
        match ast {
            Ast::Null => Value::Null,
            Ast::Str(s) => Value::Str(s.clone()),
            Ast::Num(n) => Value::Num(*n),
            Ast::Bool(b) => Value::Bool(*b),
            Ast::Path(parts) => self.resolve_path(parts),
            // Nested boolean ops evaluate to Bool.
            other => match self.eval(other) {
                EvalResult::True => Value::Bool(true),
                EvalResult::False(_) => Value::Bool(false),
            },
        }
    }

    fn resolve_path(&self, parts: &[String]) -> Value {
        if parts.is_empty() {
            return Value::Null;
        }
        match parts[0].as_str() {
            "auth" => self.resolve_auth(&parts[1..]),
            "data" => self.resolve_json(self.data, &parts[1..]),
            // On read/update/delete the route handler passes the CURRENT row
            // as `data` (the call sites bind it from `existing_row`/`pre_row`),
            // so `existing.*` is an alias for the same row — the more intuitive
            // name when gating against stored values. On insert there is no
            // prior row, so it aliases the incoming payload.
            "existing" => self.resolve_json(self.data, &parts[1..]),
            "input" => self.resolve_json(self.input, &parts[1..]),
            // `now` is the current UTC time as an ISO-8601 string, for time
            // windows. Only valid as a bare identifier (no sub-path).
            "now" if parts.len() == 1 => Value::Str(self.now.clone()),
            other => {
                // Unknown top-level — treat as null so policies fail closed
                // rather than authorizing based on unresolved identifiers.
                let _ = other;
                Value::Null
            }
        }
    }

    fn resolve_auth(&self, parts: &[String]) -> Value {
        // `auth.<field>` must name EXACTLY one field. Previously trailing
        // segments like `auth.isAdmin.foo` or `auth.userId.x.y` silently
        // resolved to the base field, over-broadening the allowed paths
        // and masking typos. Require len == 1.
        if parts.len() != 1 {
            return Value::Null;
        }
        match parts[0].as_str() {
            "userId" | "user_id" => match &self.auth.user_id {
                Some(s) => Value::Str(s.clone()),
                None => Value::Null,
            },
            "isAdmin" | "is_admin" => Value::Bool(self.auth.is_admin),
            "tenantId" | "tenant_id" => match &self.auth.tenant_id {
                Some(s) => Value::Str(s.clone()),
                None => Value::Null,
            },
            _ => Value::Null,
        }
    }

    fn resolve_json(&self, root: Option<&serde_json::Value>, parts: &[String]) -> Value {
        let mut cur = match root {
            Some(v) => v,
            None => return Value::Null,
        };
        for p in parts {
            cur = match cur.get(p) {
                Some(v) => v,
                None => return Value::Null,
            };
        }
        match cur {
            serde_json::Value::String(s) => Value::Str(s.clone()),
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Number(n) => match n.as_f64() {
                Some(f) => Value::Num(f),
                None => Value::Null,
            },
            _ => Value::Null,
        }
    }
}

fn values_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        // Numeric whenever EITHER side is a real number — so a numeric field
        // compares true to a quoted-number literal (`data.count == "3"`, the
        // pre-literals workaround) AND to a bare literal (`data.count == 3`).
        // String-vs-string stays exact (so `"0123" != "00123"`).
        (Value::Num(_), _) | (_, Value::Num(_)) => match (num_coerce(a), num_coerce(b)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        },
        (Value::Str(x), Value::Str(y)) => x == y,
        // Mixed types (e.g. str vs bool, anything vs null) are never equal.
        _ => false,
    }
}

/// Coerce a value to f64 for numeric comparison: a real number, or a string
/// that parses cleanly as one. Bool/Null/non-numeric strings → `None`.
fn num_coerce(v: &Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(*n),
        Value::Str(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Total, deny-safe ordering for `<`/`<=`/`>`/`>=`. Returns `None` when the
/// operands aren't meaningfully orderable (null, booleans, a number vs a
/// non-numeric string) — callers treat `None` as "comparison false", i.e.
/// deny. Numbers compare numerically (incl. numeric strings when the other
/// side is a number); two strings compare chronologically when both are valid
/// RFC-3339 timestamps, else lexicographically.
fn compare_values(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Num(_), _) | (_, Value::Num(_)) => num_coerce(a)?.partial_cmp(&num_coerce(b)?),
        (Value::Str(x), Value::Str(y)) => Some(
            match (
                chrono::DateTime::parse_from_rfc3339(x),
                chrono::DateTime::parse_from_rfc3339(y),
            ) {
                (Ok(dx), Ok(dy)) => dx.cmp(&dy),
                _ => x.as_str().cmp(y.as_str()),
            },
        ),
        // Null, bool, or mixed → not orderable.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::ManifestPolicy;

    // -----------------------------------------------------------------------
    // New expression grammar: &&, ||, !, parens, nested paths
    // -----------------------------------------------------------------------

    fn alice_owns(post_author: &str) -> (AuthContext, serde_json::Value) {
        let auth = AuthContext::authenticated("alice".into());
        let data = serde_json::json!({ "authorId": post_author, "status": "draft" });
        (auth, data)
    }

    #[test]
    fn conjunction_needs_both_sides() {
        let (auth, data) = alice_owns("alice");
        let r = evaluate_allow(
            "auth.userId != null && auth.userId == data.authorId",
            &auth,
            Some(&data),
            None,
        );
        assert!(matches!(r, PolicyResult::Allowed));
    }

    #[test]
    fn conjunction_fails_when_either_fails() {
        let (auth, data) = alice_owns("bob"); // not alice
        let r = evaluate_allow(
            "auth.userId != null && auth.userId == data.authorId",
            &auth,
            Some(&data),
            None,
        );
        assert!(!r.is_allowed());
    }

    #[test]
    fn disjunction_allows_admin_or_owner() {
        // Non-admin authed user; data owner is alice; check passes via owner.
        let (auth, data) = alice_owns("alice");
        let r = evaluate_allow(
            "auth.isAdmin || auth.userId == data.authorId",
            &auth,
            Some(&data),
            None,
        );
        assert!(matches!(r, PolicyResult::Allowed));

        // Admin short-circuits even when not the owner.
        let admin = AuthContext::admin();
        let r2 = evaluate_allow(
            "auth.isAdmin || auth.userId == data.authorId",
            &admin,
            Some(&data),
            None,
        );
        assert!(matches!(r2, PolicyResult::Allowed));
    }

    #[test]
    fn negation_inverts_bool() {
        let auth = AuthContext::anonymous();
        let r = evaluate_allow("!auth.isAdmin", &auth, None, None);
        assert!(matches!(r, PolicyResult::Allowed));

        let admin = AuthContext::admin();
        let r2 = evaluate_allow("!auth.isAdmin", &admin, None, None);
        assert!(!r2.is_allowed());
    }

    #[test]
    fn parentheses_group_correctly() {
        let auth = AuthContext::anonymous();
        let data = serde_json::json!({ "public": true });
        // Should evaluate as: admin OR (authed AND public)
        let expr = "auth.isAdmin || (auth.userId != null && data.public == true)";
        assert!(!evaluate_allow(expr, &auth, Some(&data), None).is_allowed());

        let authed = AuthContext::authenticated("alice".into());
        assert!(evaluate_allow(expr, &authed, Some(&data), None).is_allowed());
    }

    #[test]
    fn nested_data_path() {
        let auth = AuthContext::authenticated("alice".into());
        let data = serde_json::json!({ "author": { "id": "alice" } });
        assert!(
            evaluate_allow("auth.userId == data.author.id", &auth, Some(&data), None).is_allowed()
        );
    }

    #[test]
    fn null_comparison() {
        let auth = AuthContext::authenticated("alice".into());
        let data = serde_json::json!({ "deletedAt": null });
        assert!(evaluate_allow("data.deletedAt == null", &auth, Some(&data), None).is_allowed());
    }

    #[test]
    fn string_literal_equality() {
        let auth = AuthContext::authenticated("alice".into());
        let data = serde_json::json!({ "status": "published" });
        assert!(
            evaluate_allow("data.status == \"published\"", &auth, Some(&data), None).is_allowed()
        );
        assert!(!evaluate_allow("data.status == \"draft\"", &auth, Some(&data), None).is_allowed());
    }

    #[test]
    fn tenant_predicate() {
        let auth = AuthContext::authenticated("alice".into()).with_tenant("acme".into());
        let data = serde_json::json!({ "tenantId": "acme" });
        assert!(
            evaluate_allow("auth.tenantId == data.tenantId", &auth, Some(&data), None).is_allowed()
        );
        let data2 = serde_json::json!({ "tenantId": "other" });
        assert!(
            !evaluate_allow("auth.tenantId == data.tenantId", &auth, Some(&data2), None)
                .is_allowed()
        );
    }

    #[test]
    fn malformed_expression_denies_closed() {
        let auth = AuthContext::admin();
        let r = evaluate_allow("auth.userId == ", &auth, None, None);
        assert!(!r.is_allowed(), "parse error must fail closed");
    }

    #[test]
    fn unknown_identifier_resolves_to_null() {
        // Fail-closed: an unknown top-level identifier becomes null, so a
        // comparison against anything non-null is false.
        let auth = AuthContext::admin();
        let r = evaluate_allow("zzz.field == \"x\"", &auth, None, None);
        assert!(!r.is_allowed());
    }

    // -----------------------------------------------------------------------
    // Regression tests from the 2026 policy review.
    // -----------------------------------------------------------------------

    #[test]
    fn string_escape_n_is_newline() {
        // The scanner honors the standard escape set and preserves UTF-8:
        // `\n` stays a newline, not the letter `n`.
        let auth = AuthContext::anonymous();
        let data = serde_json::json!({ "note": "line1\nline2" });
        assert!(
            evaluate_allow("data.note == \"line1\\nline2\"", &auth, Some(&data), None).is_allowed()
        );
    }

    #[test]
    fn string_escape_unknown_is_error() {
        // An unknown escape like `\q` is a parse error that fails closed —
        // authors get loud feedback instead of a subtly-wrong rule.
        let auth = AuthContext::anonymous();
        let r = evaluate_allow("data.x == \"\\q\"", &auth, None, None);
        assert!(!r.is_allowed());
    }

    #[test]
    fn string_literal_preserves_utf8() {
        // Prior bug: `unescaped.push(b as char)` mangled `é` into garbage.
        let auth = AuthContext::anonymous();
        let data = serde_json::json!({ "name": "café" });
        assert!(evaluate_allow("data.name == \"café\"", &auth, Some(&data), None).is_allowed());
    }

    #[test]
    fn not_precedence_binds_tighter_than_eq() {
        // Prior bug: `!auth.isAdmin == false` parsed as `!(auth.isAdmin == false)`,
        // so an anonymous caller (whose isAdmin is false) would be DENIED
        // because !(false == false) == !(true) == false. With correct
        // precedence `(!auth.isAdmin) == false` is (!false) == false == true
        // only when isAdmin is true.
        let anon = AuthContext::anonymous();
        let admin = AuthContext::admin();
        // For anonymous: (!false) == false  ->  true == false -> false
        let r = evaluate_allow("!auth.isAdmin == false", &anon, None, None);
        assert!(!r.is_allowed(), "anon: (!false) == false should be false");
        // For admin: (!true) == false  ->  false == false -> true
        let r2 = evaluate_allow("!auth.isAdmin == false", &admin, None, None);
        assert!(r2.is_allowed(), "admin: (!true) == false should be true");
    }

    #[test]
    fn auth_path_rejects_extra_segments() {
        // Prior bug: `auth.isAdmin.foo` resolved as if it were `auth.isAdmin`
        // because `resolve_auth` only looked at the first segment. Now extra
        // segments return Null, which makes the comparison false.
        let admin = AuthContext::admin();
        let r = evaluate_allow("auth.isAdmin.foo == true", &admin, None, None);
        assert!(!r.is_allowed(), "extra segment must resolve to null");
        let r2 = evaluate_allow("auth.userId.x == \"anyone\"", &admin, None, None);
        assert!(!r2.is_allowed());
    }

    #[test]
    fn deep_nesting_rejected_not_panicking() {
        // Prior bug: no depth cap; 10_000 parens would stack-overflow.
        // Now the parser returns an error well before that, and
        // evaluate_allow converts it to Denied.
        let auth = AuthContext::anonymous();
        let expr = format!("{}true{}", "(".repeat(200), ")".repeat(200));
        let r = evaluate_allow(&expr, &auth, None, None);
        assert!(!r.is_allowed(), "deep nesting must deny closed, not panic");
    }

    #[test]
    fn moderate_nesting_still_parses() {
        // The cap must not break realistic expressions. 10 levels is fine.
        let auth = AuthContext::anonymous();
        let expr = format!("{}true{}", "(".repeat(10), ")".repeat(10));
        assert!(evaluate_allow(&expr, &auth, None, None).is_allowed());
    }

    #[test]
    fn parse_quoted_list_single_role() {
        assert_eq!(
            parse_quoted_string_list("\"admin\"").unwrap(),
            vec!["admin"]
        );
    }

    #[test]
    fn parse_quoted_list_two_roles() {
        assert_eq!(
            parse_quoted_string_list("'billing', 'admin'").unwrap(),
            vec!["billing", "admin"]
        );
    }

    #[test]
    fn parse_quoted_list_comma_inside_string_is_literal() {
        // This is the whole point of the fix.
        assert_eq!(
            parse_quoted_string_list("\"billing,admin\"").unwrap(),
            vec!["billing,admin"]
        );
    }

    #[test]
    fn parse_quoted_list_rejects_unquoted() {
        assert!(parse_quoted_string_list("admin").is_err());
    }

    #[test]
    fn parse_quoted_list_rejects_unterminated() {
        assert!(parse_quoted_string_list("\"unterminated").is_err());
    }

    // Synthesizes a manifest with the three policies these tests
    // exercise (one entity-read owner check, one authenticated-only
    // action, one input-owner action). Keeping it in-memory means the
    // tests don't break when the example app drops or restructures
    // its policies — the assertions describe a fixed policy shape,
    // and that shape lives here next to the assertions.
    fn test_manifest() -> AppManifest {
        let owner_read_todos = pylon_kernel::ManifestPolicy {
            name: "ownerReadTodos".into(),
            entity: Some("Todo".into()),
            allow_read: Some("auth.userId == data.authorId".into()),
            ..Default::default()
        };
        let authenticated_create = pylon_kernel::ManifestPolicy {
            name: "authenticatedCreate".into(),
            action: Some("createTodo".into()),
            allow: "auth.userId != null".into(),
            ..Default::default()
        };
        let owner_toggle = pylon_kernel::ManifestPolicy {
            name: "ownerToggle".into(),
            action: Some("toggleTodo".into()),
            allow: "auth.userId == input.authorId".into(),
            ..Default::default()
        };
        AppManifest {
            manifest_version: 1,
            name: "todo-app".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![owner_read_todos, authenticated_create, owner_toggle],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        }
    }

    #[test]
    fn engine_from_manifest() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let entity_policy_count: usize =
            engine.entity_policies_by_name.values().map(Vec::len).sum();
        assert_eq!(entity_policy_count, 1); // ownerReadTodos
        assert_eq!(engine.action_policies.len(), 2); // authenticatedCreate, ownerToggle
    }

    #[test]
    fn cached_policy_eval_is_correct_and_cache_is_populated() {
        use pylon_kernel::{AppManifest, ManifestPolicy, MANIFEST_VERSION};
        let manifest = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "t".into(),
            version: "0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![
                ManifestPolicy {
                    name: "pub".into(),
                    entity: Some("Pub".into()),
                    allow_read: Some("true".into()),
                    ..Default::default()
                },
                ManifestPolicy {
                    name: "own".into(),
                    entity: Some("Own".into()),
                    allow_read: Some("auth.userId == data.ownerId".into()),
                    ..Default::default()
                },
                ManifestPolicy {
                    name: "broken".into(),
                    entity: Some("Broken".into()),
                    allow_read: Some("auth.userId ==".into()),
                    ..Default::default()
                },
            ],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        };
        let eng = PolicyEngine::from_manifest(&manifest);
        let u1 = AuthContext::authenticated("u1".into());
        let anon = AuthContext::anonymous();

        // Public read allowed via the cached AST.
        assert!(eng.check_entity_read("Pub", &anon, None).is_allowed());

        // Row-dependent ownership: the cached AST evaluates per row.
        let mine = serde_json::json!({ "ownerId": "u1" });
        let theirs = serde_json::json!({ "ownerId": "u2" });
        assert!(eng.check_entity_read("Own", &u1, Some(&mine)).is_allowed());
        assert!(!eng
            .check_entity_read("Own", &u1, Some(&theirs))
            .is_allowed());

        // A malformed expression still denies (cached Err path == uncached).
        assert!(!eng.check_entity_read("Broken", &u1, None).is_allowed());

        // The cache was populated at construction — every distinct expression
        // compiled once, so per-row eval is a lookup, never a re-parse.
        assert!(eng.compiled.contains_key("true"));
        assert!(eng.compiled.contains_key("auth.userId == data.ownerId"));
        assert!(eng.compiled.get("auth.userId ==").unwrap().is_err());
    }

    #[test]
    fn entity_policies_bucketed_by_name_and_every_policy_enforced() {
        // #350: the by-name index must bucket ALL policies for an entity
        // together, and the lookup must be exact. A second policy on the SAME
        // entity is an additional AND-gate — if it denies, the whole denies —
        // so this also proves the index didn't drop the 2nd policy.
        use pylon_kernel::{AppManifest, ManifestPolicy, MANIFEST_VERSION};
        let manifest = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "t".into(),
            version: "0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![
                ManifestPolicy {
                    name: "doc_open".into(),
                    entity: Some("Doc".into()),
                    allow_read: Some("true".into()),
                    ..Default::default()
                },
                ManifestPolicy {
                    name: "doc_locked".into(),
                    entity: Some("Doc".into()), // 2nd gate on the SAME entity
                    allow_read: Some("false".into()),
                    ..Default::default()
                },
                ManifestPolicy {
                    name: "other".into(),
                    entity: Some("Other".into()),
                    allow_read: Some("true".into()),
                    ..Default::default()
                },
            ],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        };
        let eng = PolicyEngine::from_manifest(&manifest);

        // Exact, bucketed lookup — no per-call scan/alloc.
        assert_eq!(eng.entity_policies_for("Doc").len(), 2);
        assert_eq!(eng.entity_policies_for("Other").len(), 1);
        assert!(eng.entity_policies_for("Missing").is_empty());

        // Both Doc gates run: doc_open allows, doc_locked denies → denied.
        // If the index had dropped the 2nd same-entity policy, this would
        // wrongly allow.
        let anon = AuthContext::anonymous();
        assert!(
            !eng.check_entity_read("Doc", &anon, None).is_allowed(),
            "a second same-entity policy must still gate the read"
        );
        assert!(eng.check_entity_read("Other", &anon, None).is_allowed());
    }

    #[test]
    fn no_policies_denies_access_default_deny() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        // User entity has no policies in the test manifest. Default-
        // deny refuses access for non-admin callers — the alternative
        // (default-allow) is the Supabase RLS footgun where a forgotten
        // policy ships PII publicly.
        let anon = AuthContext::anonymous();
        let result = engine.check_entity_read("User", &anon, None);
        assert!(
            !result.is_allowed(),
            "anonymous must be denied without a policy"
        );

        let user = AuthContext::authenticated("user-1".into());
        let result = engine.check_entity_read("User", &user, None);
        assert!(
            !result.is_allowed(),
            "authenticated user must also be denied without a policy"
        );

        // Admin still bypasses — needed for migrations + Studio.
        let admin = AuthContext::admin();
        let result = engine.check_entity_read("User", &admin, None);
        assert!(
            result.is_allowed(),
            "admin must bypass the default-deny gate"
        );
    }

    #[test]
    fn admin_with_active_tenant_is_scoped_on_reads() {
        // 2026-06-04 regression: an admin (PYLON_ADMIN_EMAILS / adminField)
        // who has selected a tenant in the product UI must NOT have other
        // tenants' rows served to them. Before the fix the blanket
        // `if is_admin { Allowed }` bypass leaked every tenant's rows into
        // the admin's entity-API responses AND their browser sync replica
        // (an owner of org A saw org B's recordings / renders / viewers).
        let recording_policy = ManifestPolicy {
            name: "recordingOrg".into(),
            entity: Some("Recording".into()),
            allow_read: Some("auth.tenantId == data.orgId".into()),
            ..Default::default()
        };
        let manifest = AppManifest {
            manifest_version: 1,
            name: "t".into(),
            version: "0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![recording_policy],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        };
        let engine = PolicyEngine::from_manifest(&manifest);

        let own = serde_json::json!({ "id": "r1", "orgId": "acme" });
        let foreign = serde_json::json!({ "id": "r2", "orgId": "other" });

        // Admin WITH an active tenant: scoped exactly like a member — its
        // own org's row is allowed, a foreign org's row is DENIED. This is
        // the leak being closed.
        let admin_scoped = AuthContext::admin().with_tenant("acme".into());
        assert!(
            engine
                .check_entity_read("Recording", &admin_scoped, Some(&own))
                .is_allowed(),
            "admin-with-tenant reads its OWN tenant's row"
        );
        assert!(
            !engine
                .check_entity_read("Recording", &admin_scoped, Some(&foreign))
                .is_allowed(),
            "admin-with-tenant must NOT read a FOREIGN tenant's row (cross-tenant leak)"
        );

        // Admin with NO active tenant (operator CLI / Studio's cross-tenant
        // view): full read bypass preserved — both rows visible.
        let admin_global = AuthContext::admin();
        assert!(engine
            .check_entity_read("Recording", &admin_global, Some(&own))
            .is_allowed());
        assert!(
            engine
                .check_entity_read("Recording", &admin_global, Some(&foreign))
                .is_allowed(),
            "admin with no selected tenant keeps the cross-tenant read bypass"
        );

        // Non-admin member: unchanged — own allowed, foreign denied.
        let member = AuthContext::authenticated("u1".into()).with_tenant("acme".into());
        assert!(engine
            .check_entity_read("Recording", &member, Some(&own))
            .is_allowed());
        assert!(!engine
            .check_entity_read("Recording", &member, Some(&foreign))
            .is_allowed());

        // Scan pre-check: admin-with-tenant must NOT short-circuit-allow
        // (which would skip the per-row fence). The data-dependent tenant
        // predicate defers to per-row filtering, so the scan still
        // proceeds (Allowed) and the per-row read fence above drops foreign
        // rows. Admin-global short-circuits as before.
        assert!(
            engine
                .check_entity_scan("Recording", &admin_scoped)
                .is_allowed(),
            "tenant predicate defers to per-row filtering, so the scan proceeds"
        );
        assert!(engine
            .check_entity_scan("Recording", &admin_global)
            .is_allowed());

        // Writes keep the unconditional admin bypass (writes are
        // tenant-stamped by TenantScopePlugin and operator tooling relies
        // on it). Only the cross-tenant READ exposure is the boundary fixed
        // here — assert the write bypass is intentionally preserved.
        assert!(
            engine
                .check_entity(
                    "Recording",
                    EntityAction::Update,
                    &admin_scoped,
                    Some(&foreign)
                )
                .is_allowed(),
            "admin write bypass is intentionally preserved (not the read-leak boundary)"
        );
    }

    #[test]
    fn underscore_entities_bypass_default_deny() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let anon = AuthContext::anonymous();
        // Pylon-internal scaffolding tables (underscore-prefixed) are
        // exempt from the gate so the framework can bootstrap without
        // every app having to write a policy for `_PylonSchemaVersion`,
        // `_PylonJobs`, etc.
        let result = engine.check_entity_read("_PylonSchemaVersion", &anon, None);
        assert!(result.is_allowed());
    }

    #[test]
    fn missing_action_rule_defaults_to_deny() {
        // Build a manifest with a Todo policy that ONLY defines
        // allow_read — allow_insert/update/delete are all unset.
        // Pre-fix: writes were silently allowed. Post-fix: missing
        // rule = deny.
        let m = AppManifest {
            manifest_version: 1,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![pylon_kernel::ManifestPolicy {
                name: "todo_read_only".into(),
                entity: Some("Todo".into()),
                allow_read: Some("auth.userId != null".into()),
                ..Default::default()
            }],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        };
        let engine = PolicyEngine::from_manifest(&m);
        let user = AuthContext::authenticated("user-1".into());

        // Read works (rule is present).
        let read = engine.check_entity_read("Todo", &user, None);
        assert!(read.is_allowed());

        // Insert/update/delete have no rule → deny.
        let insert = engine.check_entity_insert("Todo", &user, None);
        assert!(
            !insert.is_allowed(),
            "insert must be denied without an allow_insert rule"
        );
        let update = engine.check_entity_update("Todo", &user, None);
        assert!(
            !update.is_allowed(),
            "update must be denied without an allow_update rule"
        );
        let delete = engine.check_entity_delete("Todo", &user, None);
        assert!(
            !delete.is_allowed(),
            "delete must be denied without an allow_delete rule"
        );
    }

    #[test]
    fn wildcard_allow_opens_intentionally_public_entity() {
        // Apps that genuinely want a public table declare a policy
        // with `allow: "true"`. This is the explicit opt-in to the
        // pre-fix default behavior, scoped to one entity.
        let m = AppManifest {
            manifest_version: 1,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![pylon_kernel::ManifestPolicy {
                name: "public_blog".into(),
                entity: Some("Post".into()),
                allow: "true".into(),
                ..Default::default()
            }],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        };
        let engine = PolicyEngine::from_manifest(&m);
        let anon = AuthContext::anonymous();
        let result = engine.check_entity_read("Post", &anon, None);
        assert!(
            result.is_allowed(),
            "wildcard allow must permit anonymous reads"
        );
    }

    #[test]
    fn auth_required_denies_anonymous() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let auth = AuthContext::anonymous();
        let result = engine.check_action("createTodo", &auth, None);
        assert!(!result.is_allowed());
    }

    #[test]
    fn auth_required_allows_authenticated() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let auth = AuthContext::authenticated("user-1".into());
        let result = engine.check_action("createTodo", &auth, None);
        assert!(result.is_allowed());
    }

    #[test]
    fn owner_check_on_entity() {
        let engine = PolicyEngine::from_manifest(&test_manifest());

        // Owner access allowed.
        let auth = AuthContext::authenticated("user-1".into());
        let data = serde_json::json!({"authorId": "user-1"});
        let result = engine.check_entity_read("Todo", &auth, Some(&data));
        assert!(result.is_allowed());

        // Non-owner denied.
        let auth = AuthContext::authenticated("user-2".into());
        let result = engine.check_entity_read("Todo", &auth, Some(&data));
        assert!(!result.is_allowed());
    }

    #[test]
    fn owner_check_on_action_input() {
        let engine = PolicyEngine::from_manifest(&test_manifest());

        // toggleTodo requires auth.userId == input.authorId
        let auth = AuthContext::authenticated("user-1".into());
        let input = serde_json::json!({"authorId": "user-1", "todoId": "todo-1"});
        let result = engine.check_action("toggleTodo", &auth, Some(&input));
        assert!(result.is_allowed());

        let auth = AuthContext::authenticated("user-2".into());
        let result = engine.check_action("toggleTodo", &auth, Some(&input));
        assert!(!result.is_allowed());
    }

    #[test]
    fn true_expression_always_allows() {
        let result = evaluate_allow("true", &AuthContext::anonymous(), None, None);
        assert!(result.is_allowed());
    }

    #[test]
    fn false_expression_always_denies() {
        let result = evaluate_allow("false", &AuthContext::anonymous(), None, None);
        assert!(!result.is_allowed());
    }

    #[test]
    fn unknown_expression_denies() {
        let result = evaluate_allow(
            "some.complex.expression",
            &AuthContext::anonymous(),
            None,
            None,
        );
        assert!(!result.is_allowed());
    }

    // -- Admin bypass --

    #[test]
    fn admin_bypasses_entity_policy() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let admin = AuthContext::admin();
        let result = engine.check_entity_read("Todo", &admin, None);
        assert!(result.is_allowed());
    }

    #[test]
    fn admin_bypasses_action_policy() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let admin = AuthContext::admin();
        let result = engine.check_action("createTodo", &admin, None);
        assert!(result.is_allowed());
    }

    #[test]
    fn non_admin_still_denied() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let anon = AuthContext::anonymous();
        let result = engine.check_action("createTodo", &anon, None);
        assert!(!result.is_allowed());
    }

    // -- Expression edge cases --

    #[test]
    fn data_field_check_without_data() {
        let result = evaluate_allow(
            "auth.userId == data.authorId",
            &AuthContext::authenticated("user-1".into()),
            None, // no data
            None,
        );
        assert!(!result.is_allowed());
    }

    #[test]
    fn input_field_check_without_input() {
        let result = evaluate_allow(
            "auth.userId == input.authorId",
            &AuthContext::authenticated("user-1".into()),
            None,
            None, // no input
        );
        assert!(!result.is_allowed());
    }

    #[test]
    fn data_field_user_mismatch() {
        let data = serde_json::json!({"authorId": "other-user"});
        let result = evaluate_allow(
            "auth.userId == data.authorId",
            &AuthContext::authenticated("user-1".into()),
            Some(&data),
            None,
        );
        assert!(!result.is_allowed());
    }

    #[test]
    fn input_field_user_mismatch() {
        let input = serde_json::json!({"authorId": "other-user"});
        let result = evaluate_allow(
            "auth.userId == input.authorId",
            &AuthContext::authenticated("user-1".into()),
            None,
            Some(&input),
        );
        assert!(!result.is_allowed());
    }

    #[test]
    fn data_field_anonymous_denied() {
        let data = serde_json::json!({"authorId": "user-1"});
        let result = evaluate_allow(
            "auth.userId == data.authorId",
            &AuthContext::anonymous(),
            Some(&data),
            None,
        );
        assert!(!result.is_allowed());
    }

    // -----------------------------------------------------------------------
    // exists() syntax — parser, evaluation, resolver wiring
    // -----------------------------------------------------------------------

    struct StubResolver {
        /// Captures each call so tests can assert on the conditions
        /// that were sent to the underlying store.
        calls: std::sync::Mutex<Vec<(String, Vec<(Vec<String>, serde_json::Value)>)>>,
        verdict: bool,
    }

    impl StubResolver {
        fn new(verdict: bool) -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
                verdict,
            }
        }
    }

    impl PolicyDataResolver for StubResolver {
        fn entity_exists(
            &self,
            entity: &str,
            conditions: &[(Vec<String>, serde_json::Value)],
        ) -> bool {
            self.calls
                .lock()
                .unwrap()
                .push((entity.to_string(), conditions.to_vec()));
            self.verdict
        }
    }

    #[test]
    fn exists_parses_single_condition() {
        // Smoke test: parse error would surface as `Policy parse error: …`
        // in the Denied reason. Allowed/Denied is determined by the
        // stub resolver. With verdict=true the policy passes.
        let resolver = StubResolver::new(true);
        let r = evaluate_allow_inner(
            "exists(OrgMember where userId == auth.userId)",
            &AuthContext::authenticated("alice".into()),
            None,
            None,
            Some(&resolver),
        );
        assert!(matches!(r, PolicyResult::Allowed), "got {r:?}");

        let calls = resolver.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "OrgMember");
        assert_eq!(
            calls[0].1,
            vec![(
                vec!["userId".to_string()],
                serde_json::Value::String("alice".into())
            )]
        );
    }

    #[test]
    fn exists_parses_multiple_conditions() {
        let resolver = StubResolver::new(true);
        let data = serde_json::json!({"orgId": "org-1"});
        let r = evaluate_allow_inner(
            "exists(OrgMember where orgId == data.orgId and userId == auth.userId)",
            &AuthContext::authenticated("alice".into()),
            Some(&data),
            None,
            Some(&resolver),
        );
        assert!(matches!(r, PolicyResult::Allowed), "got {r:?}");

        let calls = resolver.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let (entity, conditions) = &calls[0];
        assert_eq!(entity, "OrgMember");
        assert_eq!(conditions.len(), 2);
        assert_eq!(
            conditions[0],
            (
                vec!["orgId".to_string()],
                serde_json::Value::String("org-1".into())
            )
        );
        assert_eq!(
            conditions[1],
            (
                vec!["userId".to_string()],
                serde_json::Value::String("alice".into())
            )
        );
    }

    #[test]
    fn exists_denies_when_resolver_says_no_match() {
        let resolver = StubResolver::new(false);
        let r = evaluate_allow_inner(
            "exists(OrgMember where userId == auth.userId)",
            &AuthContext::authenticated("alice".into()),
            None,
            None,
            Some(&resolver),
        );
        assert!(matches!(r, PolicyResult::Denied { .. }));
    }

    #[test]
    fn exists_without_resolver_denies_with_clear_reason() {
        // If a runtime forgets to wire a resolver but the policy
        // still uses `exists()`, we must NOT silently allow — fail
        // closed with a reason that points the operator at the
        // missing wire-up.
        let r = evaluate_allow_inner(
            "exists(OrgMember where userId == auth.userId)",
            &AuthContext::authenticated("alice".into()),
            None,
            None,
            None,
        );
        match r {
            PolicyResult::Denied { reason, .. } => {
                assert!(
                    reason.contains("no data resolver wired"),
                    "unexpected reason: {reason}"
                );
            }
            _ => panic!("expected Denied, got {r:?}"),
        }
    }

    #[test]
    fn exists_composes_with_outer_and() {
        // The intended dashboard pattern: outer auth gate AND a
        // membership check. Both must pass. Outer grammar uses
        // `&&` (the bare keyword `and` is reserved for inside
        // `exists(...)`).
        let resolver = StubResolver::new(true);
        let r = evaluate_allow_inner(
            "auth.userId != null && exists(OrgMember where userId == auth.userId)",
            &AuthContext::authenticated("alice".into()),
            None,
            None,
            Some(&resolver),
        );
        assert!(matches!(r, PolicyResult::Allowed), "got {r:?}");
    }

    #[test]
    fn exists_rejects_empty_body() {
        let resolver = StubResolver::new(true);
        let r = evaluate_allow_inner(
            "exists(OrgMember where)",
            &AuthContext::authenticated("alice".into()),
            None,
            None,
            Some(&resolver),
        );
        // Parse error → Denied with a parse-error reason.
        match r {
            PolicyResult::Denied { reason, .. } => assert!(
                reason.contains("Policy parse error"),
                "unexpected reason: {reason}"
            ),
            _ => panic!("expected Denied, got {r:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Numeric literals + ordering comparisons (<, <=, >, >=)
    // -----------------------------------------------------------------------

    #[test]
    fn numeric_literal_equality() {
        let auth = AuthContext::anonymous();
        let data = serde_json::json!({ "count": 3 });
        assert!(evaluate_allow("data.count == 3", &auth, Some(&data), None).is_allowed());
        assert!(!evaluate_allow("data.count == 4", &auth, Some(&data), None).is_allowed());
    }

    #[test]
    fn numeric_ordering_int_and_float() {
        let auth = AuthContext::anonymous();
        let hi = serde_json::json!({ "priority": 5, "score": 4.9 });
        let lo = serde_json::json!({ "priority": 2, "score": 4.1 });
        assert!(evaluate_allow("data.priority >= 3", &auth, Some(&hi), None).is_allowed());
        assert!(!evaluate_allow("data.priority >= 3", &auth, Some(&lo), None).is_allowed());
        assert!(evaluate_allow("data.priority < 3", &auth, Some(&lo), None).is_allowed());
        assert!(evaluate_allow("data.score > 4.5", &auth, Some(&hi), None).is_allowed());
        assert!(!evaluate_allow("data.score > 4.5", &auth, Some(&lo), None).is_allowed());
    }

    #[test]
    fn negative_numeric_literal() {
        let auth = AuthContext::anonymous();
        let neg = serde_json::json!({ "balance": -5 });
        let pos = serde_json::json!({ "balance": 10 });
        assert!(evaluate_allow("data.balance < 0", &auth, Some(&neg), None).is_allowed());
        assert!(!evaluate_allow("data.balance < 0", &auth, Some(&pos), None).is_allowed());
    }

    #[test]
    fn numeric_field_vs_quoted_number_is_backward_compatible() {
        // Before numeric literals existed, the only way to compare a number
        // field was against a quoted string. That must still work.
        let auth = AuthContext::anonymous();
        let data = serde_json::json!({ "count": 3 });
        assert!(evaluate_allow("data.count == \"3\"", &auth, Some(&data), None).is_allowed());
    }

    #[test]
    fn string_equality_stays_exact_not_numeric() {
        // Two strings must compare exactly — NOT be coerced to numbers — so
        // zero-padded ids/zips don't collide.
        let auth = AuthContext::anonymous();
        let data = serde_json::json!({ "zip": "0123" });
        assert!(!evaluate_allow("data.zip == \"00123\"", &auth, Some(&data), None).is_allowed());
        let exact = serde_json::json!({ "zip": "00123" });
        assert!(evaluate_allow("data.zip == \"00123\"", &auth, Some(&exact), None).is_allowed());
    }

    #[test]
    fn comparison_is_deny_safe_on_unorderable_operands() {
        let auth = AuthContext::anonymous();
        // Missing field → null → comparison false (deny).
        let empty = serde_json::json!({});
        assert!(!evaluate_allow("data.missing < 3", &auth, Some(&empty), None).is_allowed());
        // Boolean vs number → not orderable → deny.
        let flag = serde_json::json!({ "flag": true });
        assert!(!evaluate_allow("data.flag < 3", &auth, Some(&flag), None).is_allowed());
        // Number vs non-numeric string → deny.
        let name = serde_json::json!({ "name": "bob" });
        assert!(!evaluate_allow("data.name >= 3", &auth, Some(&name), None).is_allowed());
    }

    #[test]
    fn string_ordering_is_chronological_for_timestamps() {
        let auth = AuthContext::anonymous();
        let row = serde_json::json!({
            "start": "2020-01-01T00:00:00Z",
            "end": "2020-12-31T23:59:59Z",
        });
        assert!(evaluate_allow("data.start < data.end", &auth, Some(&row), None).is_allowed());
        assert!(!evaluate_allow("data.end < data.start", &auth, Some(&row), None).is_allowed());
    }

    #[test]
    fn fabricated_operators_still_rejected() {
        // `in`, `+`, `ends_with` were never real — they must still fail to
        // parse (→ Denied), not silently behave.
        let auth = AuthContext::anonymous();
        let data = serde_json::json!({ "a": "x", "b": "x", "email": "a@x.com" });
        assert!(!evaluate_allow("data.a in data.b", &auth, Some(&data), None).is_allowed());
        assert!(
            !evaluate_allow("data.email ends_with \"@x.com\"", &auth, Some(&data), None)
                .is_allowed()
        );
    }

    // -----------------------------------------------------------------------
    // `now` binding — time windows
    // -----------------------------------------------------------------------

    #[test]
    fn now_enables_publish_window() {
        let auth = AuthContext::anonymous();
        // Past publish time → readable; future → not yet.
        let published = serde_json::json!({ "publishAt": "2000-01-01T00:00:00.000Z" });
        let scheduled = serde_json::json!({ "publishAt": "2999-01-01T00:00:00.000Z" });
        assert!(
            evaluate_allow("data.publishAt <= now", &auth, Some(&published), None).is_allowed()
        );
        assert!(
            !evaluate_allow("data.publishAt <= now", &auth, Some(&scheduled), None).is_allowed()
        );
    }

    #[test]
    fn now_enables_expiry_window() {
        let auth = AuthContext::anonymous();
        let live = serde_json::json!({ "expiresAt": "2999-01-01T00:00:00.000Z" });
        let expired = serde_json::json!({ "expiresAt": "2000-01-01T00:00:00.000Z" });
        assert!(evaluate_allow("data.expiresAt > now", &auth, Some(&live), None).is_allowed());
        assert!(!evaluate_allow("data.expiresAt > now", &auth, Some(&expired), None).is_allowed());
    }

    // -----------------------------------------------------------------------
    // `existing` binding — alias of the current row (fixes shipped templates)
    // -----------------------------------------------------------------------

    #[test]
    fn scan_defers_existing_and_now_read_policies_to_per_row() {
        // A read policy that references the row via `existing.*` (or a row-
        // dependent `now` comparison) must be treated as per-row filterable at
        // scan time — NOT evaluated with no row and hard-denying the whole
        // entity. Regression for the `existing` alias + comparison feature.
        use pylon_kernel::{AppManifest, ManifestPolicy, MANIFEST_VERSION};
        let manifest = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "t".into(),
            version: "0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![
                // Single owner clause: with NO row (data=None) this evaluates
                // FALSE (existing.ownerId → null), so without treating
                // `existing.` as per-row the scan would hard-deny the whole
                // entity. That's the regression this guards.
                ManifestPolicy {
                    name: "owner_read".into(),
                    entity: Some("Doc".into()),
                    allow_read: Some("existing.ownerId == auth.userId".into()),
                    ..Default::default()
                },
                ManifestPolicy {
                    name: "window".into(),
                    entity: Some("Post".into()),
                    allow_read: Some("data.publishAt <= now".into()),
                    ..Default::default()
                },
            ],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        };
        let eng = PolicyEngine::from_manifest(&manifest);
        // Authenticated user: with NO row, `existing.ownerId == auth.userId`
        // would be `null == "u1"` → false → blanket deny without the fix.
        let u1 = AuthContext::authenticated("u1".into());
        // Both defer to per-row filtering → scan is Allowed, not a blanket 403.
        assert!(eng.check_entity_scan("Doc", &u1).is_allowed());
        assert!(eng.check_entity_scan("Post", &u1).is_allowed());
    }

    #[test]
    fn existing_aliases_current_row() {
        // The b2b/consumer templates gate update/delete with
        // `existing.ownerId == auth.userId`. The router passes the current row
        // as `data`, so `existing` must resolve to it (was null → deny-all).
        let alice = AuthContext::authenticated("alice".into());
        let row = serde_json::json!({ "ownerId": "alice", "userId": "alice" });
        assert!(
            evaluate_allow("existing.ownerId == auth.userId", &alice, Some(&row), None)
                .is_allowed()
        );
        assert!(
            evaluate_allow("auth.userId == existing.userId", &alice, Some(&row), None).is_allowed()
        );

        let bob_row = serde_json::json!({ "ownerId": "bob" });
        assert!(!evaluate_allow(
            "existing.ownerId == auth.userId",
            &alice,
            Some(&bob_row),
            None
        )
        .is_allowed());
    }
}

#[cfg(test)]
mod sync_scope_tests {
    //! Replication scope: bounds WHICH rows of a synced entity reach a
    //! client's replica, without touching what that client is allowed to read.
    use super::*;
    use pylon_kernel::{AppManifest, ManifestEntity, ManifestPolicy};

    fn manifest_with(scope: Option<&str>, read: &str) -> AppManifest {
        AppManifest {
            entities: vec![ManifestEntity {
                name: "Message".into(),
                sync: true,
                sync_scope: scope.map(str::to_string),
                ..Default::default()
            }],
            policies: vec![ManifestPolicy {
                name: "msg".into(),
                entity: Some("Message".into()),
                allow: read.into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn row(room: &str) -> serde_json::Value {
        serde_json::json!({ "id": "m1", "roomId": room })
    }

    #[test]
    fn no_scope_replicates_everything_readable() {
        // The default must not change: an entity that declares no scope
        // replicates every row its read policy allows.
        let engine = PolicyEngine::from_manifest(&manifest_with(None, "true"));
        let auth = AuthContext::authenticated("alice".into());
        assert!(!engine.has_sync_scope("Message"));
        assert!(matches!(
            engine.check_sync_scope("Message", &auth, Some(&row("other"))),
            PolicyResult::Allowed
        ));
    }

    #[test]
    fn a_scope_keeps_rows_inside_it_and_drops_the_rest() {
        let engine = PolicyEngine::from_manifest(&manifest_with(
            Some("data.roomId == auth.tenantId"),
            "true",
        ));
        let mut auth = AuthContext::authenticated("alice".into());
        auth.tenant_id = Some("room-1".into());

        assert!(matches!(
            engine.check_sync_scope("Message", &auth, Some(&row("room-1"))),
            PolicyResult::Allowed
        ));
        assert!(!matches!(
            engine.check_sync_scope("Message", &auth, Some(&row("room-2"))),
            PolicyResult::Allowed
        ));
    }

    #[test]
    fn a_scope_can_never_widen_what_a_caller_reads() {
        // The whole safety argument for scopes: they are a BANDWIDTH knob.
        // Callers layer scope on top of the read policy, so a permissive
        // scope over a denying policy still yields nothing. If this ever
        // inverts, a scope becomes an access-control bypass.
        let engine = PolicyEngine::from_manifest(&manifest_with(Some("true"), "false"));
        let auth = AuthContext::authenticated("alice".into());
        assert!(matches!(
            engine.check_sync_scope("Message", &auth, Some(&row("room-1"))),
            PolicyResult::Allowed
        ));
        // …and the read policy, which the sync paths check as well, denies.
        assert!(!matches!(
            engine.check_entity_read("Message", &auth, Some(&row("room-1"))),
            PolicyResult::Allowed
        ));
    }

    #[test]
    fn a_malformed_scope_denies_rather_than_replicating_everything() {
        // Fail CLOSED. A scope that doesn't parse must not silently fall back
        // to "replicate the whole table" — that is the exact flood it exists
        // to prevent, and it would appear only in production volume.
        let engine = PolicyEngine::from_manifest(&manifest_with(Some("data.roomId =="), "true"));
        let auth = AuthContext::authenticated("alice".into());
        assert!(!matches!(
            engine.check_sync_scope("Message", &auth, Some(&row("room-1"))),
            PolicyResult::Allowed
        ));
    }

    #[test]
    fn an_empty_scope_string_is_treated_as_no_scope() {
        // `sync: { limit: 5000 }` with no `where` emits an absent/blank
        // predicate. Blank must mean "unscoped", not "deny everything" —
        // otherwise a limit-only scope silently empties the replica.
        let engine = PolicyEngine::from_manifest(&manifest_with(Some("   "), "true"));
        let auth = AuthContext::authenticated("alice".into());
        assert!(!engine.has_sync_scope("Message"));
        assert!(matches!(
            engine.check_sync_scope("Message", &auth, Some(&row("x"))),
            PolicyResult::Allowed
        ));
    }

    #[test]
    fn an_unscoped_entity_is_unaffected_by_another_entitys_scope() {
        let mut m = manifest_with(Some("data.roomId == auth.tenantId"), "true");
        m.entities.push(ManifestEntity {
            name: "Room".into(),
            sync: true,
            ..Default::default()
        });
        let engine = PolicyEngine::from_manifest(&m);
        let auth = AuthContext::authenticated("alice".into());
        assert!(engine.has_sync_scope("Message"));
        assert!(!engine.has_sync_scope("Room"));
        assert!(matches!(
            engine.check_sync_scope("Room", &auth, Some(&row("anything"))),
            PolicyResult::Allowed
        ));
    }
}

#[cfg(test)]
mod exists_memo_tests {
    //! `exists(...)` memoization. The win is per-row scans; the risk is a key
    //! that collapses two tenants into one answer, so both are pinned here.
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingResolver {
        calls: AtomicUsize,
        /// Which `referenceId` values have an active row.
        active: Vec<String>,
    }

    impl PolicyDataResolver for CountingResolver {
        fn entity_exists(
            &self,
            _entity: &str,
            conditions: &[(Vec<String>, serde_json::Value)],
        ) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            conditions.iter().any(|(path, v)| {
                path.join(".") == "referenceId"
                    && v.as_str()
                        .is_some_and(|s| self.active.contains(&s.to_string()))
            })
        }
    }

    fn engine(resolver: Arc<CountingResolver>) -> PolicyEngine {
        let m = AppManifest {
            entities: vec![pylon_kernel::ManifestEntity {
                name: "Stat".into(),
                ..Default::default()
            }],
            policies: vec![ManifestPolicy {
                name: "stat".into(),
                entity: Some("Stat".into()),
                allow: "exists(Sub where referenceId == data.orgId)".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let e = PolicyEngine::from_manifest(&m);
        e.set_resolver(resolver);
        e
    }

    fn row(org: &str) -> serde_json::Value {
        serde_json::json!({ "orgId": org })
    }

    #[test]
    fn a_scan_asks_the_store_once_instead_of_once_per_row() {
        // The Revtrail shape: one tenant gate, evaluated against every row of
        // a stat table. Without the memo this is one subquery per row.
        let resolver = Arc::new(CountingResolver {
            calls: AtomicUsize::new(0),
            active: vec!["org-1".into()],
        });
        let engine = engine(Arc::clone(&resolver));
        let auth = AuthContext::authenticated("alice".into());

        let _memo = ExistsMemo::scope();
        for _ in 0..500 {
            assert!(matches!(
                engine.check_entity_read("Stat", &auth, Some(&row("org-1"))),
                PolicyResult::Allowed
            ));
        }
        assert_eq!(
            resolver.calls.load(Ordering::SeqCst),
            1,
            "500 rows should ask the store once"
        );
    }

    #[test]
    fn two_tenants_never_share_an_answer() {
        // The thing that would make this a security bug rather than an
        // optimization: org-2 has no subscription and must still be denied
        // while org-1's cached `true` is live.
        let resolver = Arc::new(CountingResolver {
            calls: AtomicUsize::new(0),
            active: vec!["org-1".into()],
        });
        let engine = engine(Arc::clone(&resolver));
        let auth = AuthContext::authenticated("alice".into());

        let _memo = ExistsMemo::scope();
        assert!(matches!(
            engine.check_entity_read("Stat", &auth, Some(&row("org-1"))),
            PolicyResult::Allowed
        ));
        assert!(
            !matches!(
                engine.check_entity_read("Stat", &auth, Some(&row("org-2"))),
                PolicyResult::Allowed
            ),
            "a different tenant reused another tenant's cached answer"
        );
        // Two distinct questions, two store hits.
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn nothing_is_cached_outside_a_scope() {
        // Every other evaluation path — a single row read, a write check —
        // must behave exactly as before.
        let resolver = Arc::new(CountingResolver {
            calls: AtomicUsize::new(0),
            active: vec!["org-1".into()],
        });
        let engine = engine(Arc::clone(&resolver));
        let auth = AuthContext::authenticated("alice".into());

        for _ in 0..3 {
            let _ = engine.check_entity_read("Stat", &auth, Some(&row("org-1")));
        }
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn the_cache_does_not_outlive_its_scope() {
        // A subscription that lapses must be visible on the NEXT request, so
        // the map has to die with the guard.
        let resolver = Arc::new(CountingResolver {
            calls: AtomicUsize::new(0),
            active: vec!["org-1".into()],
        });
        let engine = engine(Arc::clone(&resolver));
        let auth = AuthContext::authenticated("alice".into());

        {
            let _memo = ExistsMemo::scope();
            let _ = engine.check_entity_read("Stat", &auth, Some(&row("org-1")));
            let _ = engine.check_entity_read("Stat", &auth, Some(&row("org-1")));
        }
        {
            let _memo = ExistsMemo::scope();
            let _ = engine.check_entity_read("Stat", &auth, Some(&row("org-1")));
        }
        assert_eq!(
            resolver.calls.load(Ordering::SeqCst),
            2,
            "the second scope reused the first scope's cache"
        );
    }
}
