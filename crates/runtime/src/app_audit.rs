//! Application audit log: `ctx.audit.log` / `ctx.audit.list`, and
//! automatic records for entities declared `audit: true`.
//!
//! Events go to the same append-only store as auth events
//! (`_pylon_audit_events`, SQLite or Postgres). Application actions are
//! stored as `app.<action>`, and entity writes as `entity.insert`,
//! `entity.update`, and `entity.delete`, so they can never be mistaken for
//! the built-in auth actions (`sign_in`, ...).
//!
//! Inside a mutation, events are held until the transaction commits and
//! dropped on rollback, the same as scheduled jobs: a write that did not
//! happen leaves no record.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use pylon_auth::audit::{AuditAction, AuditEvent, AuditEventBuilder, AuditQuery, AuditStore};
use pylon_auth::AuthContext;
use pylon_functions::protocol::{AuditOpMessage, AuthInfo};

const MAX_ACTION_LEN: usize = 100;
const MAX_ENTITY_LEN: usize = 100;
const MAX_ID_LEN: usize = 200;
const MAX_META_KEYS: usize = 50;
const MAX_META_KEY_LEN: usize = 100;
const MAX_META_VALUE_LEN: usize = 2000;

/// Prefix for actions logged through `ctx.audit.log`.
pub const APP_ACTION_PREFIX: &str = "app.";

thread_local! {
    /// Set for the duration of a mutation handler (see
    /// `ScheduleBufferGuard`): events buffer here and are written after
    /// COMMIT. `None` outside a mutation, where events are written at once.
    pub(crate) static MUTATION_AUDIT_BUFFER: RefCell<Option<Rc<RefCell<Vec<AuditEvent>>>>> =
        const { RefCell::new(None) };
}

/// Buffer `event` when a mutation runs on this thread, else write it.
fn buffer_or_write(store: &AuditStore, event: AuditEvent) -> Result<(), String> {
    let pending = MUTATION_AUDIT_BUFFER.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|b| b.borrow_mut().push(event.clone()))
            .is_some()
    });
    if pending {
        return Ok(());
    }
    store.try_log(&event)
}

/// Write the events a committed mutation buffered. The mutation has
/// already committed, so a failure is logged at error level.
pub(crate) fn flush_committed(store: Option<&AuditStore>, events: Vec<AuditEvent>) {
    let Some(store) = store else {
        return;
    };
    for event in events {
        if let Err(e) = store.try_log(&event) {
            tracing::error!(
                "[audit] {} event for {:?}/{:?} was lost after commit: {e}",
                event.action.as_str(),
                event.entity,
                event.entity_id
            );
        }
    }
}

/// Handle one `ctx.audit.*` frame. `auth` is the call's current auth.
pub(crate) fn handle_op(
    store: &AuditStore,
    req: &AuditOpMessage,
    auth: &AuthInfo,
) -> Result<serde_json::Value, (String, String)> {
    fn bad(msg: String) -> (String, String) {
        ("AUDIT_BAD_REQUEST".to_string(), msg)
    }
    match req.op.as_str() {
        "log" => {
            let action = req
                .action
                .as_deref()
                .ok_or_else(|| bad("audit.log requires an action".into()))?;
            check_action(action).map_err(bad)?;
            let mut builder =
                AuditEventBuilder::new(AuditAction::Custom(format!("{APP_ACTION_PREFIX}{action}")));
            if let Some(user) = &auth.user_id {
                builder = builder.actor(user.clone());
            }
            if let Some(tenant) = &auth.tenant_id {
                builder = builder.tenant(tenant.clone());
            }
            if let Some(subject) = &req.subject {
                check_len("subject", subject, MAX_ID_LEN).map_err(bad)?;
                builder = builder.user(subject.clone());
            }
            if let Some(entity) = &req.entity {
                check_len("entity", entity, MAX_ENTITY_LEN).map_err(bad)?;
                if let Some(id) = &req.entity_id {
                    check_len("entityId", id, MAX_ID_LEN).map_err(bad)?;
                }
                builder = builder.entity(entity.clone(), req.entity_id.clone());
            } else if req.entity_id.is_some() {
                return Err(bad("audit.log: entityId requires entity".into()));
            }
            if let Some(meta) = &req.meta {
                builder.metadata = meta_to_strings(meta).map_err(bad)?;
            }
            let event = builder.build();
            let id = event.id.clone();
            buffer_or_write(store, event).map_err(|e| ("AUDIT_WRITE_FAILED".to_string(), e))?;
            Ok(serde_json::json!({ "id": id }))
        }
        "list" => {
            let query = list_query(req, auth)?;
            let events = store
                .find(&query)
                .map_err(|e| ("AUDIT_READ_FAILED".to_string(), e))?;
            Ok(serde_json::Value::Array(
                events.iter().map(event_json).collect(),
            ))
        }
        other => Err(bad(format!("unknown audit op \"{other}\""))),
    }
}

/// Build the query for `ctx.audit.list`. A non-admin caller reads only
/// its own tenant, or, with no tenant, only events it performed; an
/// admin (or system) caller may name any tenant or read all of them.
fn list_query(req: &AuditOpMessage, auth: &AuthInfo) -> Result<AuditQuery, (String, String)> {
    let mut query = AuditQuery {
        actor_id: req.actor.clone(),
        entity: req.entity.clone(),
        entity_id: req.entity_id.clone(),
        action: req.action.as_deref().map(stored_action),
        before: req.before,
        limit: req.limit.unwrap_or(100),
        ..Default::default()
    };
    if auth.is_admin {
        query.tenant_id = req.tenant.clone();
        return Ok(query);
    }
    if req.tenant.is_some() && req.tenant != auth.tenant_id {
        return Err((
            "AUDIT_FORBIDDEN".to_string(),
            "only admin callers may read another tenant's audit log".to_string(),
        ));
    }
    match (&auth.tenant_id, &auth.user_id) {
        (Some(tenant), _) => query.tenant_id = Some(tenant.clone()),
        (None, Some(user)) => {
            if query.actor_id.as_ref().is_some_and(|a| a != user) {
                return Err((
                    "AUDIT_FORBIDDEN".to_string(),
                    "without a tenant, a caller may read only its own events".to_string(),
                ));
            }
            query.actor_id = Some(user.clone());
        }
        (None, None) => {
            return Err((
                "AUDIT_FORBIDDEN".to_string(),
                "audit.list requires a signed-in caller or admin".to_string(),
            ))
        }
    }
    Ok(query)
}

/// Auth actions recorded by the auth routes, stored without a prefix.
const AUTH_ACTIONS: &[&str] = &[
    "sign_in",
    "sign_out",
    "sign_in_failed",
    "sign_up",
    "password_change",
    "password_reset",
    "email_change",
    "totp_enroll",
    "totp_disable",
    "totp_backup_codes_regenerate",
    "passkey_register",
    "passkey_revoke",
    "api_key_create",
    "api_key_revoke",
    "oauth_link",
    "oauth_unlink",
    "org_create",
    "org_delete",
    "org_invite_send",
    "org_invite_accept",
    "org_member_remove",
    "org_role_change",
    "account_delete",
    "anonymous_merge",
];

/// The stored name for an action filter: `lead.export` → `app.lead.export`.
/// Names that already carry a prefix, and auth action names, are kept.
pub(crate) fn stored_action(action: &str) -> String {
    if action.starts_with(APP_ACTION_PREFIX)
        || action.starts_with("entity.")
        || AUTH_ACTIONS.contains(&action)
    {
        action.to_string()
    } else {
        format!("{APP_ACTION_PREFIX}{action}")
    }
}

/// The JSON shape `ctx.audit.list` and `GET /api/admin/audit` return.
pub(crate) fn event_json(e: &AuditEvent) -> serde_json::Value {
    serde_json::json!({
        "id": e.id,
        "createdAt": e.created_at,
        "action": e.action.as_str(),
        "actor": e.actor_id,
        "subject": e.user_id,
        "tenant": e.tenant_id,
        "entity": e.entity,
        "entityId": e.entity_id,
        "ip": e.ip,
        "success": e.success,
        "reason": e.reason,
        "meta": e.metadata,
    })
}

/// Actions are short dotted names: `lead.export`, `script.update`.
fn check_action(action: &str) -> Result<(), String> {
    if action.is_empty() || action.len() > MAX_ACTION_LEN {
        return Err(format!(
            "audit action must be 1-{MAX_ACTION_LEN} characters"
        ));
    }
    if !action
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b':'))
    {
        return Err(format!(
            "audit action \"{action}\" may contain only letters, digits, and . _ - :"
        ));
    }
    Ok(())
}

fn check_len(what: &str, value: &str, max: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > max {
        return Err(format!("audit {what} must be 1-{max} characters"));
    }
    Ok(())
}

/// Flatten `meta` to string values: strings stay as they are, anything
/// else is stored as its JSON text.
fn meta_to_strings(meta: &serde_json::Value) -> Result<HashMap<String, String>, String> {
    let obj = meta
        .as_object()
        .ok_or_else(|| "audit meta must be an object".to_string())?;
    if obj.len() > MAX_META_KEYS {
        return Err(format!("audit meta may have at most {MAX_META_KEYS} keys"));
    }
    let mut out = HashMap::with_capacity(obj.len());
    for (k, v) in obj {
        if k.is_empty() || k.len() > MAX_META_KEY_LEN {
            return Err(format!(
                "audit meta keys must be 1-{MAX_META_KEY_LEN} characters"
            ));
        }
        let text = match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if text.len() > MAX_META_VALUE_LEN {
            return Err(format!(
                "audit meta value for \"{k}\" is longer than {MAX_META_VALUE_LEN} characters"
            ));
        }
        out.insert(k.clone(), text);
    }
    Ok(out)
}

/// Records writes to entities declared `audit: true`. Runs as a plugin so
/// it sees every write path that fires plugin hooks: the entity API,
/// `ctx.db` in mutations, and nested mutations from actions.
pub struct EntityAuditPlugin {
    store: Arc<AuditStore>,
    entities: HashSet<String>,
}

thread_local! {
    /// Field names of the update in flight on this thread, captured in
    /// `before_update` (which sees the caller's patch) for the matching
    /// `after_update` (which may see the whole row).
    static UPDATE_FIELDS: RefCell<Option<(String, String, String)>> = const { RefCell::new(None) };
}

fn field_names(data: &serde_json::Value) -> Option<String> {
    let obj = data.as_object()?;
    let mut names: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "id")
        .collect();
    names.sort_unstable();
    Some(names.join(","))
}

impl EntityAuditPlugin {
    /// `None` when no entity opts in.
    pub fn from_manifest(
        manifest: &pylon_kernel::AppManifest,
        store: Arc<AuditStore>,
    ) -> Option<Self> {
        let entities: HashSet<String> = manifest
            .entities
            .iter()
            .filter(|e| e.audit)
            .map(|e| e.name.clone())
            .collect();
        (!entities.is_empty()).then_some(Self { store, entities })
    }

    fn record(
        &self,
        action: &str,
        entity: &str,
        id: &str,
        fields: Option<String>,
        auth: &AuthContext,
    ) {
        if !self.entities.contains(entity) {
            return;
        }
        let mut builder = AuditEventBuilder::new(AuditAction::Custom(action.to_string()))
            .entity(entity.to_string(), Some(id.to_string()));
        if let Some(user) = &auth.user_id {
            builder = builder.actor(user.clone());
        }
        if let Some(tenant) = &auth.tenant_id {
            builder = builder.tenant(tenant.clone());
        }
        if auth.user_id.is_none() && auth.is_admin {
            builder = builder.meta("by", "system");
        }
        if let Some(names) = fields {
            builder = builder.meta("fields", names);
        }
        if let Err(e) = buffer_or_write(&self.store, builder.build()) {
            tracing::error!("[audit] failed to record {action} on {entity}/{id}: {e}");
        }
    }
}

impl pylon_plugin::Plugin for EntityAuditPlugin {
    fn name(&self) -> &str {
        "entity-audit"
    }

    fn after_insert(&self, entity: &str, id: &str, data: &serde_json::Value, auth: &AuthContext) {
        self.record("entity.insert", entity, id, field_names(data), auth);
    }

    fn before_update(
        &self,
        entity: &str,
        id: &str,
        data: &mut serde_json::Value,
        _auth: &AuthContext,
    ) -> Result<(), pylon_plugin::PluginError> {
        if self.entities.contains(entity) {
            let names = field_names(data).unwrap_or_default();
            UPDATE_FIELDS.with(|c| *c.borrow_mut() = Some((entity.into(), id.into(), names)));
        }
        Ok(())
    }

    fn after_update(&self, entity: &str, id: &str, data: &serde_json::Value, auth: &AuthContext) {
        let captured = UPDATE_FIELDS.with(|c| {
            let mut slot = c.borrow_mut();
            match slot.as_ref() {
                Some((e, i, _)) if e == entity && i == id => slot.take().map(|(_, _, f)| f),
                _ => None,
            }
        });
        let fields = captured.or_else(|| field_names(data));
        self.record("entity.update", entity, id, fields, auth);
    }

    fn after_delete(&self, entity: &str, id: &str, auth: &AuthContext) {
        self.record("entity.delete", entity, id, None, auth);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_plugin::Plugin;

    fn store() -> Arc<AuditStore> {
        Arc::new(AuditStore::with_backend(Box::new(
            crate::audit_backend::SqliteAuditBackend::in_memory().unwrap(),
        )))
    }

    fn auth(user: Option<&str>, tenant: Option<&str>, admin: bool) -> AuthInfo {
        AuthInfo {
            user_id: user.map(str::to_string),
            is_admin: admin,
            tenant_id: tenant.map(str::to_string),
            roles: Vec::new(),
            is_guest: false,
        }
    }

    fn msg(op: &str, v: serde_json::Value) -> AuditOpMessage {
        let mut obj = v.as_object().cloned().unwrap_or_default();
        obj.insert("call_id".into(), "c1".into());
        obj.insert("op".into(), op.into());
        serde_json::from_value(serde_json::Value::Object(obj)).unwrap()
    }

    #[test]
    fn log_stamps_actor_and_tenant_and_prefixes_the_action() {
        let s = store();
        let rep = auth(Some("rep1"), Some("dealer_a"), false);
        handle_op(
            &s,
            &msg(
                "log",
                serde_json::json!({
                    "action": "lead.export",
                    "entity": "Lead",
                    "entity_id": "l1",
                    "meta": { "rows": 12, "format": "csv" },
                }),
            ),
            &rep,
        )
        .unwrap();
        let listed = handle_op(
            &s,
            &msg("list", serde_json::json!({ "action": "lead.export" })),
            &rep,
        )
        .unwrap();
        let e = &listed[0];
        assert_eq!(e["action"], "app.lead.export");
        assert_eq!(e["actor"], "rep1");
        assert_eq!(e["tenant"], "dealer_a");
        assert_eq!(e["entityId"], "l1");
        assert_eq!(e["meta"]["rows"], "12");
        assert_eq!(e["meta"]["format"], "csv");
    }

    #[test]
    fn list_is_scoped_to_the_callers_tenant() {
        let s = store();
        for (user, tenant) in [("a1", "dealer_a"), ("b1", "dealer_b")] {
            handle_op(
                &s,
                &msg("log", serde_json::json!({ "action": "script.update" })),
                &auth(Some(user), Some(tenant), false),
            )
            .unwrap();
        }
        let a = handle_op(
            &s,
            &msg("list", serde_json::json!({})),
            &auth(Some("a2"), Some("dealer_a"), false),
        )
        .unwrap();
        assert_eq!(a.as_array().unwrap().len(), 1);
        assert_eq!(a[0]["actor"], "a1");

        let err = handle_op(
            &s,
            &msg("list", serde_json::json!({ "tenant": "dealer_b" })),
            &auth(Some("a2"), Some("dealer_a"), false),
        )
        .unwrap_err();
        assert_eq!(err.0, "AUDIT_FORBIDDEN");

        let all = handle_op(
            &s,
            &msg("list", serde_json::json!({})),
            &auth(None, None, true),
        )
        .unwrap();
        assert_eq!(all.as_array().unwrap().len(), 2);

        // No tenant: only your own events.
        let solo = handle_op(
            &s,
            &msg("list", serde_json::json!({})),
            &auth(Some("b1"), None, false),
        )
        .unwrap();
        assert_eq!(solo.as_array().unwrap().len(), 1);
        assert!(handle_op(
            &s,
            &msg("list", serde_json::json!({})),
            &auth(None, None, false)
        )
        .is_err());
    }

    #[test]
    fn log_rejects_bad_input() {
        let s = store();
        let a = auth(Some("u"), Some("t"), false);
        for bad in [
            serde_json::json!({}),
            serde_json::json!({ "action": "" }),
            serde_json::json!({ "action": "has space" }),
            serde_json::json!({ "action": "x", "meta": [1] }),
            serde_json::json!({ "action": "x", "entity_id": "orphan" }),
        ] {
            let err = handle_op(&s, &msg("log", bad.clone()), &a).unwrap_err();
            assert_eq!(err.0, "AUDIT_BAD_REQUEST", "{bad}");
        }
    }

    #[test]
    fn events_logged_in_a_mutation_wait_for_commit_and_drop_on_rollback() {
        let s = store();
        let a = auth(Some("u"), Some("t"), false);
        // Rolled back: the guard drops without a flush.
        {
            let _guard = crate::datastore::ScheduleBufferGuard::enter();
            handle_op(
                &s,
                &msg("log", serde_json::json!({ "action": "rolled.back" })),
                &a,
            )
            .unwrap();
        }
        // Committed: flushed after the guard's work.
        {
            let guard = crate::datastore::ScheduleBufferGuard::enter();
            handle_op(
                &s,
                &msg("log", serde_json::json!({ "action": "committed" })),
                &a,
            )
            .unwrap();
            assert!(s
                .find(&AuditQuery {
                    limit: 10,
                    ..Default::default()
                })
                .unwrap()
                .is_empty());
            flush_committed(Some(&s), guard.take_audits());
        }
        let events = s
            .find(&AuditQuery {
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        let actions: Vec<&str> = events.iter().map(|e| e.action.as_str()).collect();
        assert_eq!(actions, vec!["app.committed"]);
    }

    #[test]
    fn entity_plugin_records_writes_with_changed_field_names() {
        let s = store();
        let manifest = pylon_kernel::AppManifest {
            entities: vec![
                pylon_kernel::ManifestEntity {
                    name: "Script".into(),
                    audit: true,
                    ..Default::default()
                },
                pylon_kernel::ManifestEntity {
                    name: "Other".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let plugin = EntityAuditPlugin::from_manifest(&manifest, Arc::clone(&s)).unwrap();
        let mut who = AuthContext::authenticated("mgr1".into());
        who.tenant_id = Some("dealer_a".into());

        plugin.after_insert(
            "Script",
            "s1",
            &serde_json::json!({ "id": "s1", "body": "hi" }),
            &who,
        );
        let mut patch = serde_json::json!({ "body": "hello" });
        plugin
            .before_update("Script", "s1", &mut patch, &who)
            .unwrap();
        // The entity API passes the full row to after_update.
        plugin.after_update(
            "Script",
            "s1",
            &serde_json::json!({ "id": "s1", "body": "hello", "title": "t" }),
            &who,
        );
        plugin.after_delete("Script", "s1", &who);
        plugin.after_insert("Other", "o1", &serde_json::json!({}), &who);

        let mut events = s
            .find(&AuditQuery {
                entity: Some("Script".into()),
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        events.reverse();
        let summary: Vec<(String, Option<String>)> = events
            .iter()
            .map(|e| {
                (
                    e.action.as_str().to_string(),
                    e.metadata.get("fields").cloned(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("entity.insert".into(), Some("body".into())),
                ("entity.update".into(), Some("body".into())),
                ("entity.delete".into(), None),
            ]
        );
        assert!(events
            .iter()
            .all(|e| e.actor_id.as_deref() == Some("mgr1")
                && e.tenant_id.as_deref() == Some("dealer_a")));
        assert!(s
            .find(&AuditQuery {
                entity: Some("Other".into()),
                limit: 10,
                ..Default::default()
            })
            .unwrap()
            .is_empty());
    }

    #[test]
    fn stored_action_prefixes_only_app_names() {
        assert_eq!(stored_action("lead.export"), "app.lead.export");
        assert_eq!(stored_action("app.lead.export"), "app.lead.export");
        assert_eq!(stored_action("entity.update"), "entity.update");
        assert_eq!(stored_action("sign_in"), "sign_in");
    }
}
