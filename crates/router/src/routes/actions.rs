//! `/api/actions/<name>` — server-validated action invocations.
//!
//! Validates against the manifest action definition (required input
//! fields, types) and runs the policy check first. The action body
//! itself is a stub that echoes input + executed: true — actual
//! action execution is wired up by the runtime layer.

use crate::{json_error, json_error_safe, json_error_with_hint, RouterContext};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    let action_name = url.strip_prefix("/api/actions/")?;
    let action_name = action_name.split('?').next().unwrap_or(action_name);
    if method != HttpMethod::Post {
        return Some((
            405,
            json_error("METHOD_NOT_ALLOWED", "Actions require POST"),
        ));
    }

    let input: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return Some((
                400,
                json_error_safe(
                    "INVALID_JSON",
                    "Invalid request body",
                    &format!("Invalid JSON: {e}"),
                ),
            ));
        }
    };

    let policy_check = ctx
        .policy_engine
        .check_action(action_name, ctx.auth_ctx, Some(&input));
    if !policy_check.is_allowed() {
        if let pylon_policy::PolicyResult::Denied {
            policy_name,
            reason,
        } = policy_check
        {
            // Don't leak the raw allow expression to the client — it
            // reveals role names, field names, and the access-control
            // model. Log server-side so operators can debug.
            tracing::warn!(
                "[policy] action \"{action_name}\" denied by \"{policy_name}\": {reason}"
            );
            return Some((403, json_error("POLICY_DENIED", "Access denied by policy")));
        }
    }

    let manifest = ctx.store.manifest();
    let action_def = manifest.actions.iter().find(|a| a.name == action_name);
    if action_def.is_none() {
        let available: Vec<&str> = manifest.actions.iter().map(|a| a.name.as_str()).collect();
        return Some((
            404,
            json_error_with_hint(
                "ACTION_NOT_FOUND",
                &format!("Unknown action: \"{action_name}\""),
                &format!("Available actions: [{}]", available.join(", ")),
            ),
        ));
    }
    let action_def = action_def.unwrap();

    let input_obj = input.as_object();
    for field in &action_def.input {
        if !field.optional {
            let has_field = input_obj
                .and_then(|o| o.get(&field.name))
                .map(|v| !v.is_null())
                .unwrap_or(false);
            if !has_field {
                let required: Vec<String> = action_def
                    .input
                    .iter()
                    .filter(|f| !f.optional)
                    .map(|f| format!("{}: {}", f.name, f.field_type))
                    .collect();
                return Some((
                    400,
                    json_error_with_hint(
                        "ACTION_MISSING_INPUT",
                        &format!(
                            "Required input field \"{}\" (type: {}) is missing for action \"{}\"",
                            field.name, field.field_type, action_name
                        ),
                        &format!("Required fields: [{}]", required.join(", ")),
                    ),
                ));
            }
        }
    }

    Some((
        200,
        serde_json::json!({
            "action": action_name,
            "input": input,
            "executed": true,
        })
        .to_string(),
    ))
}
