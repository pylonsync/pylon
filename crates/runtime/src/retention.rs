//! Data retention: delete rows of entities declared with `retention`
//! once they pass their retention period.
//!
//! A system job runs [`sweep`] every hour (see server.rs). Each row is
//! deleted through the entity pipeline (plugin hooks, change log, sync
//! broadcast, CRDT and search cleanup), and each deletion is recorded in
//! the audit log as `retention.delete`, whether or not the entity is
//! audited.

use std::sync::Arc;

use pylon_auth::audit::{AuditAction, AuditEventBuilder, AuditStore};
use pylon_kernel::{AppManifest, ManifestEntity, ManifestRetention};

use crate::entity_writer::{EntityWriter, WriteError};
use crate::RuntimeError;

/// Rows read per query while sweeping one entity.
const PAGE: usize = 200;

/// How a rule's `field` is compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    /// ISO-8601 text (SQLite) or TIMESTAMPTZ (Postgres).
    Datetime,
    /// Unix milliseconds.
    EpochMillis,
}

/// A validated retention rule.
#[derive(Debug, Clone)]
pub struct RetentionRule {
    pub entity: String,
    pub field: String,
    kind: FieldKind,
    /// `None`: `field` is the expiry time itself.
    pub after_secs: Option<u64>,
    pub hold: Option<String>,
}

/// Result of one sweep.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct SweepReport {
    pub deleted: u64,
    /// Expired rows kept because their hold field is true.
    pub held: u64,
    /// Deletes that failed (logged); the row is tried again next sweep.
    pub failed: u64,
}

/// Parse "90d", "12h", "30m", "45s", "2w", or "1y" (365 days) into seconds.
pub fn parse_retention_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    let unit = s.chars().last()?;
    let digits = &s[..s.len() - unit.len_utf8()];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    let mult = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        'w' => 7 * 86_400,
        'y' => 365 * 86_400,
        _ => return None,
    };
    n.checked_mul(mult).filter(|&secs| secs > 0)
}

/// Validate every entity's retention rule. Errors list each problem so a
/// deploy fails at boot instead of deleting the wrong rows or none.
pub fn rules_from_manifest(manifest: &AppManifest) -> Result<Vec<RetentionRule>, RuntimeError> {
    let mut rules = Vec::new();
    let mut problems = Vec::new();
    for ent in &manifest.entities {
        let Some(r) = &ent.retention else { continue };
        match rule_for(ent, r) {
            Ok(rule) => rules.push(rule),
            Err(e) => problems.push(format!("{}: {e}", ent.name)),
        }
    }
    if !problems.is_empty() {
        return Err(RuntimeError {
            code: "RETENTION_MANIFEST_INVALID".into(),
            message: format!("Invalid retention rules:\n  - {}", problems.join("\n  - ")),
        });
    }
    Ok(rules)
}

fn rule_for(ent: &ManifestEntity, r: &ManifestRetention) -> Result<RetentionRule, String> {
    let field = ent
        .fields
        .iter()
        .find(|f| f.name == r.field)
        .ok_or_else(|| format!("retention field \"{}\" does not exist", r.field))?;
    let kind = match field.field_type.as_str() {
        "datetime" => FieldKind::Datetime,
        "int" | "float" => FieldKind::EpochMillis,
        other => {
            return Err(format!(
            "retention field \"{}\" must be datetime, or int/float unix milliseconds (got {other})",
            r.field
        ))
        }
    };
    let after_secs = match &r.after {
        None => None,
        Some(a) => Some(parse_retention_duration(a).ok_or_else(|| {
            format!("retention after \"{a}\" is not a duration like \"90d\", \"12h\", or \"1y\"")
        })?),
    };
    if let Some(hold) = &r.hold {
        let hf = ent
            .fields
            .iter()
            .find(|f| &f.name == hold)
            .ok_or_else(|| format!("retention hold field \"{hold}\" does not exist"))?;
        if hf.field_type != "bool" {
            return Err(format!("retention hold field \"{hold}\" must be bool"));
        }
    }
    Ok(RetentionRule {
        entity: ent.name.clone(),
        field: r.field.clone(),
        kind,
        after_secs,
        hold: r.hold.clone(),
    })
}

impl RetentionRule {
    /// The filter value: rows with `field` at or before it are expired.
    fn cutoff(&self, now_secs: u64) -> serde_json::Value {
        let cutoff = now_secs.saturating_sub(self.after_secs.unwrap_or(0));
        match self.kind {
            FieldKind::Datetime => {
                serde_json::Value::String(pylon_kernel::util::epoch_to_iso(cutoff))
            }
            FieldKind::EpochMillis => serde_json::json!(cutoff.saturating_mul(1000)),
        }
    }
}

/// Delete every expired row of every rule. `now_secs` is the current time
/// (a parameter so tests can move it).
pub fn sweep(
    writer: &EntityWriter,
    audit: &AuditStore,
    rules: &[RetentionRule],
    now_secs: u64,
) -> Result<SweepReport, RuntimeError> {
    let mut report = SweepReport::default();
    for rule in rules {
        sweep_rule(writer, audit, rule, now_secs, &mut report)?;
    }
    Ok(report)
}

fn sweep_rule(
    writer: &EntityWriter,
    audit: &AuditStore,
    rule: &RetentionRule,
    now_secs: u64,
    report: &mut SweepReport,
) -> Result<(), RuntimeError> {
    let cutoff = rule.cutoff(now_secs);
    // Rows this pass keeps (held or failed) stay in the result set, so the
    // next page starts after them.
    let mut skip: u64 = 0;
    loop {
        let filter = serde_json::json!({
            rule.field.as_str(): { "$lte": cutoff },
            "$order": { rule.field.as_str(): "asc" },
            "$limit": PAGE,
            "$offset": skip,
        });
        let rows = writer.runtime().query_filtered(&rule.entity, &filter)?;
        let count = rows.len();
        for row in rows {
            let Some(id) = row.get("id").and_then(|v| v.as_str()) else {
                skip += 1;
                continue;
            };
            if rule
                .hold
                .as_deref()
                .is_some_and(|h| row.get(h).and_then(|v| v.as_bool()) == Some(true))
            {
                report.held += 1;
                skip += 1;
                continue;
            }
            match writer.delete(&rule.entity, id) {
                Ok(()) => {
                    report.deleted += 1;
                    let event =
                        AuditEventBuilder::new(AuditAction::Custom("retention.delete".into()))
                            .entity(rule.entity.clone(), Some(id.to_string()))
                            .meta("field", rule.field.clone())
                            .meta("cutoff", cutoff.to_string().trim_matches('"').to_string())
                            .meta("by", "system");
                    let mut event = match row.get("tenantId").and_then(|v| v.as_str()) {
                        Some(t) => event.tenant(t),
                        None => event,
                    };
                    if let Some(after) = rule.after_secs {
                        event = event.meta("after_secs", after.to_string());
                    }
                    if let Err(e) = audit.try_log(&event.build()) {
                        tracing::error!(
                            "[retention] deleted {}/{id} but could not record it: {e}",
                            rule.entity
                        );
                    }
                }
                Err(WriteError::Refused(why)) if why == "no such row" => {
                    // Deleted concurrently: nothing to do, and it left the
                    // result set.
                }
                Err(e) => {
                    tracing::warn!("[retention] could not delete {}/{id}: {e:?}", rule.entity);
                    report.failed += 1;
                    skip += 1;
                }
            }
        }
        if count < PAGE {
            break;
        }
    }
    Ok(())
}

/// Run a sweep now; used by the hourly job and the admin endpoint.
pub fn run(
    writer: &Arc<EntityWriter>,
    audit: &Arc<AuditStore>,
    rules: &[RetentionRule],
) -> Result<SweepReport, RuntimeError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    sweep(writer, audit, rules, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_auth::audit::AuditQuery;
    use pylon_kernel::{ManifestEntity, ManifestField};

    const DAY: u64 = 86_400;
    /// A fixed "now": 2026-06-01T00:00:00Z.
    const NOW: u64 = 1_780_272_000;

    fn field(name: &str, ty: &str) -> ManifestField {
        ManifestField {
            name: name.into(),
            field_type: ty.into(),
            optional: true,
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

    fn manifest() -> AppManifest {
        AppManifest {
            manifest_version: 1,
            name: "retention".into(),
            version: "0.1.0".into(),
            entities: vec![
                ManifestEntity {
                    name: "Recording".into(),
                    fields: vec![
                        field("createdAt", "datetime"),
                        field("tenantId", "string"),
                        field("legalHold", "bool"),
                    ],
                    crdt: false,
                    retention: Some(ManifestRetention {
                        field: "createdAt".into(),
                        after: Some("30d".into()),
                        hold: Some("legalHold".into()),
                    }),
                    ..Default::default()
                },
                ManifestEntity {
                    name: "Transcript".into(),
                    fields: vec![field("deleteAfter", "float")],
                    crdt: false,
                    retention: Some(ManifestRetention {
                        field: "deleteAfter".into(),
                        after: None,
                        hold: None,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    fn writer(rt: Arc<crate::Runtime>) -> EntityWriter {
        let m = rt.manifest().clone();
        EntityWriter::new(
            Arc::clone(&rt),
            Arc::new(pylon_sync::ChangeLog::new()),
            Arc::new(pylon_router::NoopNotifier),
            Arc::new(pylon_policy::PolicyEngine::from_manifest(&m)),
            Arc::new(pylon_plugin::PluginRegistry::new(m)),
        )
    }

    fn audit() -> Arc<AuditStore> {
        Arc::new(AuditStore::with_backend(Box::new(
            crate::audit_backend::SqliteAuditBackend::in_memory().unwrap(),
        )))
    }

    #[test]
    fn durations_parse() {
        assert_eq!(parse_retention_duration("30d"), Some(30 * DAY));
        assert_eq!(parse_retention_duration("12h"), Some(12 * 3600));
        assert_eq!(parse_retention_duration("2w"), Some(14 * DAY));
        assert_eq!(parse_retention_duration("1y"), Some(365 * DAY));
        for bad in ["", "d", "30", "0d", "-1d", "30x", "3.5d"] {
            assert_eq!(parse_retention_duration(bad), None, "{bad}");
        }
    }

    #[test]
    fn invalid_rules_fail_validation() {
        let mut m = manifest();
        m.entities[0].retention = Some(ManifestRetention {
            field: "tenantId".into(),
            after: Some("forever".into()),
            hold: Some("createdAt".into()),
        });
        let err = rules_from_manifest(&m).unwrap_err();
        assert_eq!(err.code, "RETENTION_MANIFEST_INVALID");
        assert!(err.message.contains("must be datetime"), "{}", err.message);
        m.entities[0].retention = Some(ManifestRetention {
            field: "missing".into(),
            after: None,
            hold: None,
        });
        assert!(rules_from_manifest(&m)
            .unwrap_err()
            .message
            .contains("does not exist"));
        assert_eq!(rules_from_manifest(&manifest()).unwrap().len(), 2);
    }

    fn iso(secs: u64) -> String {
        pylon_kernel::util::epoch_to_iso(secs)
    }

    fn seed_and_sweep(rt: Arc<crate::Runtime>) {
        let old = rt
            .insert(
                "Recording",
                &serde_json::json!({ "createdAt": iso(NOW - 40 * DAY), "tenantId": "dealer_a", "legalHold": false }),
            )
            .unwrap();
        let held = rt
            .insert(
                "Recording",
                &serde_json::json!({ "createdAt": iso(NOW - 40 * DAY), "tenantId": "dealer_a", "legalHold": true }),
            )
            .unwrap();
        let recent = rt
            .insert(
                "Recording",
                &serde_json::json!({ "createdAt": iso(NOW - 5 * DAY), "tenantId": "dealer_a", "legalHold": false }),
            )
            .unwrap();
        let expired = rt
            .insert(
                "Transcript",
                &serde_json::json!({ "deleteAfter": ((NOW - 1) * 1000) as f64 }),
            )
            .unwrap();
        let live = rt
            .insert(
                "Transcript",
                &serde_json::json!({ "deleteAfter": ((NOW + DAY) * 1000) as f64 }),
            )
            .unwrap();

        let w = writer(Arc::clone(&rt));
        let a = audit();
        let rules = rules_from_manifest(rt.manifest()).unwrap();
        let report = sweep(&w, &a, &rules, NOW).unwrap();
        assert_eq!(
            report,
            SweepReport {
                deleted: 2,
                held: 1,
                failed: 0
            }
        );

        assert!(rt.get_by_id("Recording", &old).unwrap().is_none());
        assert!(rt.get_by_id("Recording", &held).unwrap().is_some());
        assert!(rt.get_by_id("Recording", &recent).unwrap().is_some());
        assert!(rt.get_by_id("Transcript", &expired).unwrap().is_none());
        assert!(rt.get_by_id("Transcript", &live).unwrap().is_some());

        let events = a
            .find(&AuditQuery {
                action: Some("retention.delete".into()),
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(events.len(), 2);
        let rec = events
            .iter()
            .find(|e| e.entity.as_deref() == Some("Recording"))
            .unwrap();
        assert_eq!(rec.entity_id.as_deref(), Some(old.as_str()));
        assert_eq!(rec.tenant_id.as_deref(), Some("dealer_a"));

        // A second sweep has nothing new to delete.
        let again = sweep(&w, &a, &rules, NOW).unwrap();
        assert_eq!(
            again,
            SweepReport {
                deleted: 0,
                held: 1,
                failed: 0
            }
        );
    }

    #[test]
    fn sqlite_sweep_deletes_expired_rows_keeps_held_ones_and_audits() {
        let rt = Arc::new(crate::Runtime::in_memory(manifest()).unwrap());
        seed_and_sweep(rt);
    }

    #[test]
    fn postgres_sweep_deletes_expired_rows_keeps_held_ones_and_audits() {
        let Ok(url) = std::env::var("PYLON_TEST_PG_URL") else {
            eprintln!("skipping: PYLON_TEST_PG_URL not set");
            return;
        };
        let m = manifest();
        let mut adapter =
            pylon_storage::postgres::live::LivePostgresAdapter::connect(&url).unwrap();
        for t in ["Recording", "Transcript"] {
            adapter
                .exec_raw(&format!("DROP TABLE IF EXISTS \"{t}\" CASCADE"))
                .unwrap();
        }
        let plan = adapter.plan_from_live(&m).unwrap();
        adapter.apply_plan(&plan).unwrap();
        let rt = Arc::new(crate::Runtime::open_postgres(&url, m).unwrap());
        seed_and_sweep(rt);
    }
}
