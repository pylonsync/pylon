//! Shared server-side mutation pipeline.
//!
//! Every write surface (entity REST, `/api/sync/push`, `/api/batch`,
//! `/api/transact`, `/api/link`, `/api/unlink`, function-runtime
//! auto-broadcast store) funnels through [`apply_mutation`] so the
//! same ordering invariants hold uniformly:
//!
//! 1. row preload (Update/Delete/Link/Unlink need the existing row
//!    for policy authz; Insert has no preload)
//! 2. policy check (action-specific: insert / update / delete)
//! 3. plugin `before_*` hook (can deny or mutate the payload)
//! 4. store mutation
//! 5. post-write reread (full row for insert/update; pre-delete
//!    snapshot is captured at step 1 for delete)
//! 6. typed change-log append via [`ChangeRecord`]
//! 7. JSON + CRDT broadcast (skipped when reread missed)
//! 8. plugin `after_*` hook
//!
//! Callers receive a [`MutationOutcome`] carrying the assigned seq,
//! the full row (None when the reread missed), and the change kind.
//! Per-op envelopes (`/api/sync/push`, `/api/batch`) map this
//! directly to their wire shape.

use crate::{broadcast_change_with_crdt, ChangeNotifier, PluginHookOps, RouterContext};
use pylon_auth::AuthContext;
use pylon_http::DataStore;
use pylon_policy::{PolicyEngine, PolicyResult};
use pylon_sync::{ChangeKind, ChangeLog, ChangeRecord};

/// One mutation the pipeline knows how to apply. Carries everything
/// the pipeline needs — no further request parsing happens inside.
#[derive(Debug)]
pub enum MutationOp<'a> {
    Insert {
        entity: &'a str,
        data: &'a serde_json::Value,
    },
    Update {
        entity: &'a str,
        row_id: &'a str,
        data: &'a serde_json::Value,
    },
    Delete {
        entity: &'a str,
        row_id: &'a str,
    },
    /// FK link: sets `relation` on `entity/row_id` to `target_id`.
    /// Emits an Update change event; the policy gate is
    /// `check_entity_update`, never `check_entity_write` (which
    /// confusingly delegates to EntityAction::Insert).
    Link {
        entity: &'a str,
        row_id: &'a str,
        relation: &'a str,
        target_id: &'a str,
    },
    /// FK unlink: clears `relation` on `entity/row_id`. Same Update
    /// semantics as Link.
    Unlink {
        entity: &'a str,
        row_id: &'a str,
        relation: &'a str,
    },
}

impl<'a> MutationOp<'a> {
    pub fn entity(&self) -> &'a str {
        match self {
            MutationOp::Insert { entity, .. }
            | MutationOp::Update { entity, .. }
            | MutationOp::Delete { entity, .. }
            | MutationOp::Link { entity, .. }
            | MutationOp::Unlink { entity, .. } => entity,
        }
    }

    /// Known row id when supplied up front — everything except
    /// Insert. Insert mints its id during the store call.
    pub fn row_id(&self) -> Option<&'a str> {
        match self {
            MutationOp::Update { row_id, .. }
            | MutationOp::Delete { row_id, .. }
            | MutationOp::Link { row_id, .. }
            | MutationOp::Unlink { row_id, .. } => Some(row_id),
            MutationOp::Insert { .. } => None,
        }
    }

    pub fn change_kind(&self) -> ChangeKind {
        match self {
            MutationOp::Insert { .. } => ChangeKind::Insert,
            MutationOp::Update { .. } | MutationOp::Link { .. } | MutationOp::Unlink { .. } => {
                ChangeKind::Update
            }
            MutationOp::Delete { .. } => ChangeKind::Delete,
        }
    }
}

/// Successful result of a mutation pipeline run.
#[derive(Debug)]
pub struct MutationOutcome {
    /// Sequence number assigned by `ChangeLog::record`. Zero when the
    /// reread missed and no event was appended.
    pub seq: u64,
    /// The id the row landed under. For Insert this is the server-
    /// minted id; for other variants it's the caller-supplied id
    /// echoed back.
    pub row_id: String,
    /// Full row payload. `Some` on Insert/Update success when the
    /// reread caught the row, `Some` on Delete when the pre-delete
    /// snapshot was capturable, `None` only when a reread missed
    /// (concurrent delete or read-side outage).
    pub row: Option<serde_json::Value>,
    /// Echo of the change kind. Per-op envelopes use this when they
    /// surface "what happened" to clients.
    pub kind: ChangeKind,
}

/// Failure modes. Each maps to an HTTP status + JSON error envelope
/// at the route layer via [`error_response`].
#[derive(Debug)]
pub enum MutationError {
    /// Policy gate denied. Caller maps to 403.
    PolicyDenied { policy: String, reason: String },
    /// Plugin `before_*` hook denied. Caller maps to the hook's
    /// returned status (typically 400/403).
    Hook {
        status: u16,
        code: String,
        message: String,
    },
    /// Row id didn't resolve. Caller maps to 404. Used by Update,
    /// Delete, Link, Unlink — Insert never hits this.
    NotFound,
    /// Store-layer error. Caller maps to 400/500 based on the code.
    Store { code: String, message: String },
}

/// Slice of [`RouterContext`] the pipeline actually touches.
/// Constructed at the call site instead of taking the full context so
/// lower-level callers (workflow runner, transact fanout, function
/// runtime auto-broadcast store) can drive the pipeline without a
/// full router setup.
pub struct MutationCtx<'a> {
    pub store: &'a dyn DataStore,
    pub change_log: &'a ChangeLog,
    pub notifier: &'a dyn ChangeNotifier,
    pub policy: &'a PolicyEngine,
    pub plugin_hooks: &'a dyn PluginHookOps,
    pub auth: &'a AuthContext,
    /// Skip the per-op policy gate. Set by admin-bypass surfaces
    /// (`/api/batch`, server-side migrations driven by ops tools)
    /// that have already established trust at the route layer
    /// via `require_admin`. Plugin hooks + reread + broadcast still
    /// run unchanged — only the `check_entity_*` step is skipped.
    pub bypass_policy: bool,
}

impl<'a> MutationCtx<'a> {
    pub fn from_router(ctx: &'a RouterContext<'a>) -> Self {
        Self {
            store: ctx.store,
            change_log: ctx.change_log,
            notifier: ctx.notifier,
            policy: ctx.policy_engine,
            plugin_hooks: ctx.plugin_hooks,
            auth: ctx.auth_ctx,
            bypass_policy: false,
        }
    }

    /// Variant for admin-bypass paths (`/api/batch`, transact). Skips
    /// the per-op policy gate; everything else (plugin hooks, reread,
    /// broadcast, after-hooks) runs identically to the standard
    /// pipeline.
    pub fn from_router_admin(ctx: &'a RouterContext<'a>) -> Self {
        Self {
            bypass_policy: true,
            ..Self::from_router(ctx)
        }
    }
}

/// Run a mutation through the full pipeline.
///
/// On success: store written, change log appended, broadcasts emitted
/// (JSON + CRDT for insert/update), after-hook fired. On error:
/// nothing was broadcast; the store may or may not have been written
/// depending on which step failed.
pub fn apply_mutation(ctx: &MutationCtx, op: MutationOp) -> Result<MutationOutcome, MutationError> {
    let entity = op.entity().to_string();
    let kind = op.change_kind();

    // Pre-load the existing row when policy needs to authorize
    // against authoritative state. Pre-delete snapshot lives here
    // too — it's the data we emit on the change event for row-scoped
    // read policies on subscribers.
    //
    // Update/Link/Unlink: row missing → NotFound short-circuit; you
    // can't authorize against a row that doesn't exist.
    //
    // Delete: row missing is NOT short-circuited here. The store's
    // delete may be idempotent (returns Ok(true) for a missing row),
    // and the route layer already gated policy. Run the delete and
    // let `store.delete`'s result drive NotFound.
    let pre_row: Option<serde_json::Value> = match &op {
        MutationOp::Insert { .. } => None,
        MutationOp::Update { row_id, .. }
        | MutationOp::Link { row_id, .. }
        | MutationOp::Unlink { row_id, .. } => match ctx.store.get_by_id(&entity, row_id) {
            Ok(Some(row)) => Some(row),
            Ok(None) => return Err(MutationError::NotFound),
            Err(e) => {
                return Err(MutationError::Store {
                    code: e.code,
                    message: e.message,
                });
            }
        },
        MutationOp::Delete { row_id, .. } => match ctx.store.get_by_id(&entity, row_id) {
            Ok(Some(row)) => Some(row),
            Ok(None) => None,
            Err(e) => {
                return Err(MutationError::Store {
                    code: e.code,
                    message: e.message,
                });
            }
        },
    };

    // Policy gate. Insert sees the caller's data; everything else
    // sees the authoritative pre_row. Delete uses
    // `check_entity_delete` so the action-specific predicate runs.
    //
    // Skipped entirely for admin-bypass surfaces (see
    // `MutationCtx::bypass_policy`) — those have already established
    // trust at the route layer via `require_admin`.
    if !ctx.bypass_policy {
        let policy_result = match &op {
            MutationOp::Insert { data, .. } => {
                ctx.policy
                    .check_entity_insert(&entity, ctx.auth, Some(data))
            }
            MutationOp::Update { .. } | MutationOp::Link { .. } | MutationOp::Unlink { .. } => ctx
                .policy
                .check_entity_update(&entity, ctx.auth, pre_row.as_ref()),
            MutationOp::Delete { .. } => {
                ctx.policy
                    .check_entity_delete(&entity, ctx.auth, pre_row.as_ref())
            }
        };
        if let PolicyResult::Denied {
            policy_name,
            reason,
        } = policy_result
        {
            return Err(MutationError::PolicyDenied {
                policy: policy_name,
                reason,
            });
        }
    }

    // before_* hook + store mutation. Each arm returns
    // (row_id, post_row, after_hook_fallback). `post_row` drives the
    // broadcast — when the reread misses, post_row is None and we
    // skip the broadcast. `after_hook_fallback` is the data we pass
    // to the plugin's `after_*` callback when post_row is missing:
    // for insert/update this is the post-hook caller payload so
    // plugins still observe SOMETHING after a write, matching the
    // pre-pipeline contract.
    let (row_id, post_row, after_fallback): (
        String,
        Option<serde_json::Value>,
        Option<serde_json::Value>,
    ) = match op {
        MutationOp::Insert { entity: ent, data } => {
            let mut hook_data = data.clone();
            if let Err((status, code, message)) =
                ctx.plugin_hooks
                    .before_insert(ent, &mut hook_data, ctx.auth)
            {
                return Err(MutationError::Hook {
                    status,
                    code,
                    message,
                });
            }
            let id = ctx
                .store
                .insert(ent, &hook_data)
                .map_err(|e| MutationError::Store {
                    code: e.code,
                    message: e.message,
                })?;
            let post = reread(ctx.store, ent, &id);
            (id, post, Some(hook_data))
        }
        MutationOp::Update {
            entity: ent,
            row_id,
            data,
        } => {
            let mut hook_data = data.clone();
            if let Err((status, code, message)) =
                ctx.plugin_hooks
                    .before_update(ent, row_id, &mut hook_data, ctx.auth)
            {
                return Err(MutationError::Hook {
                    status,
                    code,
                    message,
                });
            }
            match ctx.store.update(ent, row_id, &hook_data) {
                Ok(true) => {
                    let post = reread(ctx.store, ent, row_id);
                    (row_id.to_string(), post, Some(hook_data))
                }
                Ok(false) => return Err(MutationError::NotFound),
                Err(e) => {
                    return Err(MutationError::Store {
                        code: e.code,
                        message: e.message,
                    });
                }
            }
        }
        MutationOp::Delete {
            entity: ent,
            row_id,
        } => {
            if let Err((status, code, message)) =
                ctx.plugin_hooks.before_delete(ent, row_id, ctx.auth)
            {
                return Err(MutationError::Hook {
                    status,
                    code,
                    message,
                });
            }
            match ctx.store.delete(ent, row_id) {
                Ok(true) => (row_id.to_string(), pre_row.clone(), None),
                Ok(false) => return Err(MutationError::NotFound),
                Err(e) => {
                    return Err(MutationError::Store {
                        code: e.code,
                        message: e.message,
                    });
                }
            }
        }
        MutationOp::Link {
            entity: ent,
            row_id,
            relation,
            target_id,
        } => {
            // Plugins observe FK mutations as Updates; pass a
            // structured marker so plugins that want to special-case
            // can. Most just treat it as any other update.
            let mut hook_data = serde_json::json!({
                "_pylon_link": { "relation": relation, "target_id": target_id },
            });
            if let Err((status, code, message)) =
                ctx.plugin_hooks
                    .before_update(ent, row_id, &mut hook_data, ctx.auth)
            {
                return Err(MutationError::Hook {
                    status,
                    code,
                    message,
                });
            }
            match ctx.store.link(ent, row_id, relation, target_id) {
                Ok(true) => {
                    let post = reread(ctx.store, ent, row_id);
                    (row_id.to_string(), post, Some(hook_data))
                }
                Ok(false) => return Err(MutationError::NotFound),
                Err(e) => {
                    return Err(MutationError::Store {
                        code: e.code,
                        message: e.message,
                    });
                }
            }
        }
        MutationOp::Unlink {
            entity: ent,
            row_id,
            relation,
        } => {
            let mut hook_data = serde_json::json!({
                "_pylon_unlink": { "relation": relation },
            });
            if let Err((status, code, message)) =
                ctx.plugin_hooks
                    .before_update(ent, row_id, &mut hook_data, ctx.auth)
            {
                return Err(MutationError::Hook {
                    status,
                    code,
                    message,
                });
            }
            match ctx.store.unlink(ent, row_id, relation) {
                Ok(true) => {
                    let post = reread(ctx.store, ent, row_id);
                    (row_id.to_string(), post, Some(hook_data))
                }
                Ok(false) => return Err(MutationError::NotFound),
                Err(e) => {
                    return Err(MutationError::Store {
                        code: e.code,
                        message: e.message,
                    });
                }
            }
        }
    };

    // Typed change-log append. Insert/Update need the post_row;
    // Delete needs the pre-delete snapshot. When post_row is None for
    // insert/update (concurrent delete), skip both append and
    // broadcast — the concurrent delete fires its own event.
    //
    // Update events carry `prev_data` so the per-subscriber broadcast
    // filter can synthesize a Delete tombstone for clients whose
    // read policy passed pre-update but denies post-update (e.g.
    // ownership transferred away). Without this, those subscribers
    // would silently lose visibility of the update and keep the
    // stale row in their local replica.
    // No-op suppression: an update that left the row identical has
    // nothing to replicate — skip the change-log append and the
    // broadcast (see `update_is_noop`). The write itself succeeded, so
    // the after_update hook still fires and the caller still gets the
    // row back; `seq: 0` matches the concurrent-delete path's "no event
    // was logged" convention (a 0 seq never advances client cursors).
    if matches!(kind, ChangeKind::Update) {
        if let Some(post) = post_row.as_ref() {
            if crate::update_is_noop(ctx.store.manifest(), &entity, pre_row.as_ref(), post) {
                if let Some(row) = post_row.as_ref().or(after_fallback.as_ref()) {
                    ctx.plugin_hooks
                        .after_update(&entity, &row_id, row, ctx.auth);
                }
                return Ok(MutationOutcome {
                    seq: 0,
                    row_id,
                    row: post_row,
                    kind,
                });
            }
        }
    }

    let record: Option<ChangeRecord> = match kind {
        ChangeKind::Insert => post_row
            .as_ref()
            .map(|r| ChangeRecord::Insert { row: r.clone() }),
        ChangeKind::Update => post_row.as_ref().map(|r| ChangeRecord::Update {
            row: r.clone(),
            prev: pre_row.clone(),
        }),
        ChangeKind::Delete => Some(ChangeRecord::Delete {
            snapshot: post_row.clone(),
        }),
    };

    if let Some(record) = record {
        let event = ctx.change_log.record(&entity, &row_id, record);
        broadcast_change_with_crdt(
            ctx.notifier,
            ctx.store,
            event.seq,
            &entity,
            &row_id,
            event.kind.clone(),
            event.data.as_ref(),
            event.prev_data.as_ref(),
        );

        // after_* hook: prefer the post-write row, fall back to the
        // hook_data (post-`before_*`-hook caller payload) so plugins
        // still observe SOMETHING after a successful write even when
        // the reread missed. Delete after-hook doesn't take a row.
        let hook_payload = post_row.as_ref().or(after_fallback.as_ref());
        match event.kind {
            ChangeKind::Insert => {
                if let Some(row) = hook_payload {
                    ctx.plugin_hooks
                        .after_insert(&entity, &row_id, row, ctx.auth);
                }
            }
            ChangeKind::Update => {
                if let Some(row) = hook_payload {
                    ctx.plugin_hooks
                        .after_update(&entity, &row_id, row, ctx.auth);
                }
            }
            ChangeKind::Delete => {
                ctx.plugin_hooks.after_delete(&entity, &row_id, ctx.auth);
            }
        }

        Ok(MutationOutcome {
            seq: event.seq,
            row_id,
            row: post_row,
            kind: event.kind,
        })
    } else {
        // Insert/Update reread missed and this isn't a delete. The
        // store mutation succeeded; fire the after-hook with the
        // hook_data fallback so plugins still observe the write, but
        // skip the broadcast (the concurrent delete will fire its
        // own event). No change-log entry — without a row to ship
        // there's nothing meaningful to log.
        if let Some(row) = after_fallback.as_ref() {
            match kind {
                ChangeKind::Insert => {
                    ctx.plugin_hooks
                        .after_insert(&entity, &row_id, row, ctx.auth);
                }
                ChangeKind::Update => {
                    ctx.plugin_hooks
                        .after_update(&entity, &row_id, row, ctx.auth);
                }
                ChangeKind::Delete => {
                    ctx.plugin_hooks.after_delete(&entity, &row_id, ctx.auth);
                }
            }
        }
        Ok(MutationOutcome {
            seq: 0,
            row_id,
            row: None,
            kind,
        })
    }
}

/// Re-read a row after a successful insert/update. The three
/// outcomes flatten to `Option`:
///
/// - `Ok(Some(row))` → full row available; broadcast it.
/// - `Ok(None)`      → row was concurrently deleted between our
///                     write and the reread. Skip the broadcast.
/// - `Err(_)`        → read-side outage. Skip the broadcast rather
///                     than falling back to the caller's partial
///                     payload (which would trip the per-tenant
///                     policy filter on every subscriber).
fn reread(store: &dyn DataStore, entity: &str, id: &str) -> Option<serde_json::Value> {
    match store.get_by_id(entity, id) {
        Ok(Some(row)) => Some(row),
        Ok(None) => {
            tracing::warn!(
                "[mutate] post-write reread returned None for {entity}/{id} — concurrent delete; skipping broadcast"
            );
            None
        }
        Err(e) => {
            tracing::warn!(
                "[mutate] post-write reread failed for {entity}/{id} ({}): skipping broadcast",
                e.message
            );
            None
        }
    }
}

/// Map a [`MutationError`] to a (status, error code, message) triple
/// for the route-layer error envelope. Routes that want richer
/// behavior (e.g. include the deny policy name) can read the
/// `MutationError` directly instead.
pub fn error_response(err: &MutationError) -> (u16, String, String) {
    match err {
        MutationError::PolicyDenied { reason, .. } => (
            403,
            "POLICY_DENIED".to_string(),
            format!("Access denied by policy: {reason}"),
        ),
        MutationError::Hook {
            status,
            code,
            message,
        } => (*status, code.clone(), message.clone()),
        MutationError::NotFound => (404, "NOT_FOUND".to_string(), "Row not found".to_string()),
        MutationError::Store { code, message } => {
            let status = match code.as_str() {
                "ENTITY_NOT_FOUND" | "ROW_NOT_FOUND" => 404,
                "NOT_SUPPORTED" | "INVALID_INPUT" => 400,
                _ => 400,
            };
            (status, code.clone(), message.clone())
        }
    }
}
