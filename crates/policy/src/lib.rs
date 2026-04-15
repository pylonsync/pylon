use agentdb_auth::AuthContext;
use agentdb_core::{AppManifest, ManifestPolicy};

// ---------------------------------------------------------------------------
// Policy evaluation
// ---------------------------------------------------------------------------

/// Result of a policy check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyResult {
    Allowed,
    Denied { policy_name: String, reason: String },
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
    entity_policies: Vec<ManifestPolicy>,
    action_policies: Vec<ManifestPolicy>,
}

impl PolicyEngine {
    /// Build a policy engine from a manifest.
    pub fn from_manifest(manifest: &AppManifest) -> Self {
        let mut entity_policies = Vec::new();
        let mut action_policies = Vec::new();

        for policy in &manifest.policies {
            if policy.entity.is_some() {
                entity_policies.push(policy.clone());
            }
            if policy.action.is_some() {
                action_policies.push(policy.clone());
            }
        }

        Self {
            entity_policies,
            action_policies,
        }
    }

    /// Check if an entity read is allowed for the given auth context.
    /// `data` is the row being accessed (for field-level checks).
    pub fn check_entity_read(
        &self,
        entity_name: &str,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        // Admin bypasses all policies.
        if auth.is_admin {
            return PolicyResult::Allowed;
        }

        let policies: Vec<&ManifestPolicy> = self
            .entity_policies
            .iter()
            .filter(|p| p.entity.as_deref() == Some(entity_name))
            .collect();

        if policies.is_empty() {
            return PolicyResult::Allowed;
        }

        for policy in &policies {
            match evaluate_allow(&policy.allow, auth, data, None) {
                PolicyResult::Denied { .. } => {
                    return PolicyResult::Denied {
                        policy_name: policy.name.clone(),
                        reason: format!(
                            "Policy \"{}\" denied: {}",
                            policy.name, policy.allow
                        ),
                    };
                }
                PolicyResult::Allowed => {}
            }
        }

        PolicyResult::Allowed
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
            match evaluate_allow(&policy.allow, auth, None, input) {
                PolicyResult::Denied { .. } => {
                    return PolicyResult::Denied {
                        policy_name: policy.name.clone(),
                        reason: format!(
                            "Policy \"{}\" denied: {}",
                            policy.name, policy.allow
                        ),
                    };
                }
                PolicyResult::Allowed => {}
            }
        }

        PolicyResult::Allowed
    }
}

/// Evaluate an `allow` expression against auth context and data.
fn evaluate_allow(
    expr: &str,
    auth: &AuthContext,
    data: Option<&serde_json::Value>,
    input: Option<&serde_json::Value>,
) -> PolicyResult {
    let expr = expr.trim();

    // "true" — always allowed
    if expr == "true" {
        return PolicyResult::Allowed;
    }

    // "false" — always denied
    if expr == "false" {
        return PolicyResult::Denied {
            policy_name: String::new(),
            reason: "Expression is false".into(),
        };
    }

    // "auth.userId != null" — requires authenticated user
    if expr == "auth.userId != null" {
        return if auth.is_authenticated() {
            PolicyResult::Allowed
        } else {
            PolicyResult::Denied {
                policy_name: String::new(),
                reason: "Authentication required".into(),
            }
        };
    }

    // "auth.userId == data.<field>" — user must match a data field
    if let Some(field) = expr.strip_prefix("auth.userId == data.") {
        let field = field.trim();
        if let Some(data) = data {
            let data_value = data.get(field).and_then(|v| v.as_str());
            let user_id = auth.user_id.as_deref();
            return if user_id.is_some() && user_id == data_value {
                PolicyResult::Allowed
            } else {
                PolicyResult::Denied {
                    policy_name: String::new(),
                    reason: format!("User does not match data.{field}"),
                }
            };
        }
        // No data to check against — deny.
        return PolicyResult::Denied {
            policy_name: String::new(),
            reason: format!("No data available to check data.{field}"),
        };
    }

    // "auth.userId == input.<field>" — user must match an input field
    if let Some(field) = expr.strip_prefix("auth.userId == input.") {
        let field = field.trim();
        if let Some(input) = input {
            let input_value = input.get(field).and_then(|v| v.as_str());
            let user_id = auth.user_id.as_deref();
            return if user_id.is_some() && user_id == input_value {
                PolicyResult::Allowed
            } else {
                PolicyResult::Denied {
                    policy_name: String::new(),
                    reason: format!("User does not match input.{field}"),
                }
            };
        }
        return PolicyResult::Denied {
            policy_name: String::new(),
            reason: format!("No input available to check input.{field}"),
        };
    }

    // Unknown expression — deny with explanation.
    PolicyResult::Denied {
        policy_name: String::new(),
        reason: format!("Cannot evaluate expression: \"{expr}\""),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use agentdb_core::ManifestPolicy;

    fn test_manifest() -> AppManifest {
        serde_json::from_str(include_str!("../../../examples/todo-app/agentdb.manifest.json"))
            .unwrap()
    }

    #[test]
    fn engine_from_manifest() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        assert_eq!(engine.entity_policies.len(), 1); // ownerReadTodos
        assert_eq!(engine.action_policies.len(), 2); // authenticatedCreate, ownerToggle
    }

    #[test]
    fn no_policies_allows_access() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let auth = AuthContext::anonymous();
        // User entity has no policies.
        let result = engine.check_entity_read("User", &auth, None);
        assert!(result.is_allowed());
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
}
