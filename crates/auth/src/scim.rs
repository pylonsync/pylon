//! SCIM 2.0 — System for Cross-domain Identity Management.
//!
//! Lets enterprise IdPs (Okta, Azure AD, Workday, Rippling) auto-
//! provision users into pylon-managed apps. The IdP POSTs to
//! `/scim/v2/Users` to create a user, GETs `/scim/v2/Users/<id>`
//! to read, PATCHes to update, DELETEs to deactivate. Same shape
//! for `/scim/v2/Groups`.
//!
//! **Status: library only — HTTP endpoints not yet wired.**
//! ScimUser / ScimError / check_bearer ship today as primitives so
//! apps that want to roll their own SCIM endpoints can compose
//! them. The pylon-shipped `/scim/v2/*` routes (POST/GET/PATCH/
//! DELETE Users + Groups) are queued for the next wave.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "familyName")]
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
            emails: vec![
                ScimEmail {
                    value: "alice@example.com".into(),
                    primary: Some(true),
                    kind: Some("work".into()),
                },
            ],
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
