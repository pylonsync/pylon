//! SCIM 2.0 — System for Cross-domain Identity Management.
//!
//! Lets enterprise IdPs (Okta, Azure AD, Workday, Rippling) auto-
//! provision users into pylon-managed apps. The IdP POSTs to
//! `/scim/v2/Users` to create a user, GETs `/scim/v2/Users/<id>`
//! to read, PATCHes to update, DELETEs to deactivate. Same shape
//! for `/scim/v2/Groups`.
//!
//! **Status: library + HTTP endpoints (Users only).** Pylon ships
//! `POST /scim/v2/Users`, `GET /scim/v2/Users` (with `userName eq`
//! filter support), `GET /scim/v2/Users/{id}`, `PATCH /scim/v2/Users/{id}`,
//! `PUT /scim/v2/Users/{id}`, `DELETE /scim/v2/Users/{id}` (soft),
//! plus the SCIM service-discovery trio at
//! `/scim/v2/{ServiceProviderConfig, Schemas, ResourceTypes}` that
//! Okta + Azure AD probe on connect. Groups + array-path PATCH
//! filters (`emails[primary eq true].value`) deferred — most IdPs
//! work without those.
//!
//! Auth: SCIM endpoints accept a static bearer token configured via
//! `PYLON_SCIM_TOKEN`. IdPs configure this once when they connect.
//!
//! Spec: <https://datatracker.ietf.org/doc/html/rfc7644>
//!
//! Pylon's SCIM mapping:
//!   - SCIM `userName` → User row's `email`
//!   - SCIM `name.formatted` → User row's `displayName`
//!   - SCIM `active=false` → soft-delete (set `deletedAt` on User row;
//!     app decides whether to hard-delete)
//!
//! The endpoint wiring lives in `routes/auth.rs`. This module just
//! provides the request/response type definitions and the
//! field-level mapping helpers.

use serde::{Deserialize, Serialize};

/// SCIM User schema (subset). Most IdPs send a much fuller object
/// — pylon ignores anything we don't model. `extra` captures it
/// for round-trip on PATCH.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimUser {
    /// SCIM "id" — the IdP-assigned identifier. Pylon uses its own
    /// User row id internally and stores SCIM id as `scimId`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Universal SCIM identifier — typically the email.
    #[serde(rename = "userName")]
    pub user_name: String,
    /// Whether the IdP considers this user active. `false` is the
    /// soft-delete signal.
    #[serde(default = "default_active")]
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<ScimName>,
    /// First email is treated as primary if `primary` flag missing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<ScimEmail>,
    // `displayName` (not `display_name`) on the wire — SCIM spec
    // names every attribute in camelCase. Pre-fix the response
    // sent `display_name`, which Okta + Azure AD silently ignored
    // because the SCIM schema doesn't define that attribute.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "displayName"
    )]
    pub display_name: Option<String>,
    /// SCIM schemas array — must include at least
    /// `urn:ietf:params:scim:schemas:core:2.0:User`.
    #[serde(default = "default_user_schemas")]
    pub schemas: Vec<String>,
}

fn default_active() -> bool {
    true
}

fn default_user_schemas() -> Vec<String> {
    vec!["urn:ietf:params:scim:schemas:core:2.0:User".into()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimName {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formatted: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "givenName")]
    pub given_name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "familyName"
    )]
    pub family_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimEmail {
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
    pub kind: Option<String>,
}

impl ScimUser {
    /// Pull the primary email — `primary=true` first, else the first
    /// element, else fall back to `userName`.
    pub fn primary_email(&self) -> &str {
        self.emails
            .iter()
            .find(|e| e.primary == Some(true))
            .map(|e| e.value.as_str())
            .or_else(|| self.emails.first().map(|e| e.value.as_str()))
            .unwrap_or(&self.user_name)
    }

    /// Best-effort display name — `displayName` first, else
    /// `name.formatted`, else `<given> <family>`.
    pub fn pretty_display_name(&self) -> String {
        if let Some(d) = &self.display_name {
            return d.clone();
        }
        if let Some(name) = &self.name {
            if let Some(f) = &name.formatted {
                return f.clone();
            }
            let parts: Vec<&str> = [&name.given_name, &name.family_name]
                .iter()
                .filter_map(|o| o.as_deref())
                .collect();
            if !parts.is_empty() {
                return parts.join(" ");
            }
        }
        self.user_name.clone()
    }
}

/// SCIM error response shape — RFC 7644 §3.12.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimError {
    pub schemas: Vec<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "scimType")]
    pub scim_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ScimError {
    pub fn new(status: u16, detail: &str) -> Self {
        Self {
            schemas: vec!["urn:ietf:params:scim:api:messages:2.0:Error".into()],
            status: status.to_string(),
            scim_type: None,
            detail: Some(detail.to_string()),
        }
    }
}

/// SCIM list response (RFC 7644 §3.4.2).
#[derive(Debug, Clone, Serialize)]
pub struct ScimListResponse<T> {
    pub schemas: Vec<String>,
    #[serde(rename = "totalResults")]
    pub total_results: usize,
    #[serde(rename = "Resources")]
    pub resources: Vec<T>,
}

impl<T> ScimListResponse<T> {
    pub fn new(resources: Vec<T>) -> Self {
        Self {
            schemas: vec!["urn:ietf:params:scim:api:messages:2.0:ListResponse".into()],
            total_results: resources.len(),
            resources,
        }
    }
}

/// SCIM PATCH request body (RFC 7644 §3.5.2). IdPs use this to
/// partially-update a User instead of replacing the whole resource.
#[derive(Debug, Clone, Deserialize)]
pub struct ScimPatchRequest {
    #[serde(default)]
    pub schemas: Vec<String>,
    #[serde(rename = "Operations")]
    pub operations: Vec<ScimPatchOp>,
}

/// One operation inside a SCIM PATCH. The `op` field is one of
/// "add" / "replace" / "remove" (case-insensitive per RFC 7644). The
/// `path` field is a SCIM attribute name with optional dot-nested
/// sub-attribute (`name.formatted`). For "remove" `value` is absent.
/// Pylon doesn't yet model array-filter paths (`emails[primary eq
/// true].value`); patches that use them get an OperationNotSupported
/// SCIM error from the route handler.
#[derive(Debug, Clone, Deserialize)]
pub struct ScimPatchOp {
    pub op: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

/// Map a PATCH operation onto Pylon's User row fields. Returns the
/// `(column, value)` pair to feed into a `DataStore::update` call,
/// or `Err` for unsupported paths so the route can surface a
/// SCIM-shaped error instead of silently dropping the op.
///
/// Supported paths (case-insensitive on the SCIM side, lowercased
/// for the column lookup):
///   - `userName`              → `email`
///   - `displayName`           → `displayName`
///   - `name.formatted`        → `displayName` (most IdPs send this
///                               instead of the top-level field)
///   - `active`                → `scimActive` (bool)
///   - `externalId`            → `scimExternalId`
///
/// Anything else returns Err with the offending path so the caller
/// can build a 400 ScimError.
pub fn patch_op_to_field_update(
    op: &ScimPatchOp,
) -> Result<(&'static str, serde_json::Value), String> {
    let raw_path = op.path.as_deref().unwrap_or_default();
    // Reject path-filter syntax (`emails[primary eq true].value`)
    // outright — the route surfaces this to the IdP as a typed
    // SCIM error. Implementing it properly needs a full filter
    // parser; deferring keeps the MVP shape predictable.
    if raw_path.contains('[') {
        return Err(format!(
            "array-filter PATCH paths not supported: {raw_path}"
        ));
    }
    let column = match raw_path.to_ascii_lowercase().as_str() {
        "username" => "email",
        "displayname" | "name.formatted" => "displayName",
        "active" => "scimActive",
        "externalid" => "scimExternalId",
        other => {
            return Err(format!("unsupported PATCH path: {other}"));
        }
    };
    let kind = op.op.to_ascii_lowercase();
    let value = match kind.as_str() {
        "remove" => serde_json::Value::Null,
        "add" | "replace" => op
            .value
            .clone()
            .ok_or_else(|| format!("PATCH {kind} requires a value"))?,
        other => return Err(format!("unknown PATCH op: {other}")),
    };
    Ok((column, value))
}

/// Parse a SCIM filter string into the subset Pylon supports.
///
/// Real SCIM filters (RFC 7644 §3.4.2.2) are an expression grammar
/// with `and`/`or`/`not`, comparators (`eq`/`ne`/`co`/`sw`/`ew`/`gt`/
/// `lt`/`pr`), and parenthesization. Implementing the full grammar
/// is a multi-day project; instead pylon recognizes the ONE shape
/// IdPs actually emit during day-to-day provisioning:
///
///   `userName eq "<value>"`     — user-existence probe before POST
///
/// Everything else returns None so the route falls through to an
/// unfiltered list. Operators who need full filter support can
/// open an issue; the 90% case (Okta, Azure AD, Workday all check
/// userName equality first) is handled.
pub fn parse_username_eq_filter(filter: &str) -> Option<String> {
    let trimmed = filter.trim();
    // Case-insensitive prefix match on `userName eq`. SCIM
    // attribute names are case-insensitive per RFC 7643.
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower.strip_prefix("username eq ")?;
    // Find the same offset in the original (preserves case of the
    // quoted value, though for emails it doesn't matter).
    let value_part = &trimmed[trimmed.len() - rest.len()..];
    // Value must be quoted with double-quotes.
    let value = value_part.strip_prefix('"')?.strip_suffix('"')?;
    Some(value.to_string())
}

/// Validate a bearer token against `PYLON_SCIM_TOKEN`. Returns
/// `true` only if the env var is set + the bearer matches via
/// constant-time compare.
pub fn check_bearer(authorization_header: Option<&str>) -> bool {
    let Some(header) = authorization_header else {
        return false;
    };
    let Some(presented) = header.strip_prefix("Bearer ") else {
        return false;
    };
    let Ok(expected) = std::env::var("PYLON_SCIM_TOKEN") else {
        return false;
    };
    if expected.is_empty() {
        return false;
    }
    crate::constant_time_eq(presented.trim().as_bytes(), expected.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> ScimUser {
        ScimUser {
            id: Some("scim-1".into()),
            user_name: "alice@example.com".into(),
            active: true,
            name: Some(ScimName {
                formatted: Some("Alice Liddell".into()),
                given_name: Some("Alice".into()),
                family_name: Some("Liddell".into()),
            }),
            emails: vec![ScimEmail {
                value: "alice@example.com".into(),
                primary: Some(true),
                kind: Some("work".into()),
            }],
            display_name: None,
            schemas: default_user_schemas(),
        }
    }

    #[test]
    fn primary_email_falls_back_to_userName() {
        let mut u = alice();
        u.emails.clear();
        assert_eq!(u.primary_email(), "alice@example.com");
    }

    #[test]
    fn primary_email_picks_primary_flag() {
        let mut u = alice();
        u.emails = vec![
            ScimEmail {
                value: "alt@example.com".into(),
                primary: Some(false),
                kind: None,
            },
            ScimEmail {
                value: "main@example.com".into(),
                primary: Some(true),
                kind: None,
            },
        ];
        assert_eq!(u.primary_email(), "main@example.com");
    }

    #[test]
    fn display_name_pretty_formatted() {
        let u = alice();
        assert_eq!(u.pretty_display_name(), "Alice Liddell");
    }

    #[test]
    fn display_name_falls_back_to_givenName_familyName() {
        let mut u = alice();
        u.name.as_mut().unwrap().formatted = None;
        assert_eq!(u.pretty_display_name(), "Alice Liddell");
    }

    #[test]
    fn deserialize_okta_shape() {
        let raw = r#"{
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
            "userName": "user@okta.example",
            "active": true,
            "name": {"givenName": "Bob", "familyName": "Smith"},
            "emails": [{"value": "user@okta.example", "primary": true}]
        }"#;
        let u: ScimUser = serde_json::from_str(raw).expect("parse");
        assert_eq!(u.user_name, "user@okta.example");
        assert!(u.active);
        assert_eq!(u.primary_email(), "user@okta.example");
        assert_eq!(u.pretty_display_name(), "Bob Smith");
    }

    #[test]
    fn list_response_serializes_with_totalResults() {
        let list = ScimListResponse::new(vec![alice()]);
        let json = serde_json::to_string(&list).unwrap();
        assert!(json.contains("\"totalResults\":1"));
        assert!(json.contains("\"Resources\""));
    }

    #[test]
    fn patch_op_replace_username_maps_to_email() {
        let op = ScimPatchOp {
            op: "replace".into(),
            path: Some("userName".into()),
            value: Some(serde_json::json!("new@example.com")),
        };
        let (col, val) = patch_op_to_field_update(&op).unwrap();
        assert_eq!(col, "email");
        assert_eq!(val, serde_json::json!("new@example.com"));
    }

    #[test]
    fn patch_op_replace_name_formatted_maps_to_displayName() {
        // Okta + Azure AD both send name.formatted instead of the
        // top-level `displayName` field.
        let op = ScimPatchOp {
            op: "Replace".into(),
            path: Some("name.formatted".into()),
            value: Some(serde_json::json!("Alice Liddell")),
        };
        let (col, val) = patch_op_to_field_update(&op).unwrap();
        assert_eq!(col, "displayName");
        assert_eq!(val, serde_json::json!("Alice Liddell"));
    }

    #[test]
    fn patch_op_replace_active_handles_bool() {
        // IdPs use `active=false` for the deactivate flow.
        let op = ScimPatchOp {
            op: "replace".into(),
            path: Some("active".into()),
            value: Some(serde_json::json!(false)),
        };
        let (col, val) = patch_op_to_field_update(&op).unwrap();
        assert_eq!(col, "scimActive");
        assert_eq!(val, serde_json::json!(false));
    }

    #[test]
    fn patch_op_remove_emits_null() {
        let op = ScimPatchOp {
            op: "remove".into(),
            path: Some("displayName".into()),
            value: None,
        };
        let (col, val) = patch_op_to_field_update(&op).unwrap();
        assert_eq!(col, "displayName");
        assert!(val.is_null());
    }

    #[test]
    fn patch_op_unsupported_path_errors() {
        let op = ScimPatchOp {
            op: "replace".into(),
            path: Some("addresses".into()),
            value: Some(serde_json::json!([])),
        };
        let err = patch_op_to_field_update(&op).unwrap_err();
        assert!(err.contains("unsupported"));
    }

    #[test]
    fn patch_op_array_filter_path_rejected() {
        let op = ScimPatchOp {
            op: "replace".into(),
            path: Some(r#"emails[primary eq true].value"#.into()),
            value: Some(serde_json::json!("new@example.com")),
        };
        let err = patch_op_to_field_update(&op).unwrap_err();
        assert!(err.contains("array-filter"));
    }

    #[test]
    fn parse_username_eq_filter_extracts_value() {
        // The exact shape Okta + Azure AD probe with.
        assert_eq!(
            parse_username_eq_filter(r#"userName eq "alice@example.com""#),
            Some("alice@example.com".to_string())
        );
        // Case-insensitive attribute name per RFC 7643.
        assert_eq!(
            parse_username_eq_filter(r#"USERNAME EQ "bob@example.com""#),
            Some("bob@example.com".to_string())
        );
        // Anything else returns None — caller falls through to
        // unfiltered list.
        assert_eq!(parse_username_eq_filter("active eq true"), None);
        assert_eq!(parse_username_eq_filter(r#"userName co "alice""#), None);
    }

    #[test]
    fn scim_patch_request_deserializes_okta_shape() {
        let raw = r#"{
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [
                {"op": "Replace", "path": "active", "value": false},
                {"op": "Replace", "path": "name.formatted", "value": "Alice Liddell"}
            ]
        }"#;
        let req: ScimPatchRequest = serde_json::from_str(raw).expect("parse");
        assert_eq!(req.operations.len(), 2);
        assert_eq!(req.operations[0].path.as_deref(), Some("active"));
        assert_eq!(req.operations[1].path.as_deref(), Some("name.formatted"));
    }

    #[test]
    fn check_bearer_constant_time_compare() {
        // Without the env var set, all checks fail.
        std::env::remove_var("PYLON_SCIM_TOKEN");
        assert!(!check_bearer(Some("Bearer something")));
        std::env::set_var("PYLON_SCIM_TOKEN", "secret-test-token-7c4f");
        assert!(!check_bearer(Some("Bearer wrong")));
        assert!(!check_bearer(None));
        assert!(!check_bearer(Some("Basic abc")));
        assert!(check_bearer(Some("Bearer secret-test-token-7c4f")));
        std::env::remove_var("PYLON_SCIM_TOKEN");
    }
}
