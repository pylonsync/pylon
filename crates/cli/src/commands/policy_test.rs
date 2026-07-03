//! `pylon policy test` — dry-run a policy expression against an
//! explicit auth context and row, without a running app.
//!
//! Agents (and humans) write policy expressions blind and discover the
//! semantics at runtime as 403s. This makes the evaluator itself
//! addressable:
//!
//! ```text
//! pylon policy test 'auth.userId == data.ownerId' \
//!     --auth userId=u1 --row '{"ownerId":"u1"}'
//! # → ALLOW
//!
//! pylon policy test 'auth.tenantId == data.orgId' \
//!     --auth userId=u1,tenantId=t1 --row '{"orgId":"t2"}' --json
//! # → {"result":"deny","reason":"..."}
//! ```
//!
//! Exit codes are scriptable: 0 = allowed, 1 = denied, 64 = usage /
//! invalid JSON. Same tokenizer + parser + evaluator as production
//! enforcement (see `pylon_policy::evaluate_expression`), so this is a
//! faithful oracle, not an approximation.

use pylon_auth::AuthContext;
use pylon_kernel::ExitCode;
use pylon_policy::PolicyResult;

use crate::output;

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    // Dispatch: `pylon policy test <expr> [...]`. Only `test` exists
    // today; the subcommand slot keeps room for `policy explain` etc.
    let mut it = args.iter().skip(1); // skip "policy"
    let sub = it.next().map(String::as_str);
    if sub != Some("test") {
        print_usage();
        return ExitCode::Usage;
    }

    let mut expr: Option<&str> = None;
    let mut auth_kvs: Option<&str> = None;
    let mut row_json: Option<&str> = None;
    let mut input_json: Option<&str> = None;

    let rest: Vec<&String> = it.collect();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--auth" if i + 1 < rest.len() => {
                auth_kvs = Some(rest[i + 1]);
                i += 2;
            }
            "--row" if i + 1 < rest.len() => {
                row_json = Some(rest[i + 1]);
                i += 2;
            }
            "--input" if i + 1 < rest.len() => {
                input_json = Some(rest[i + 1]);
                i += 2;
            }
            "--json" => {
                i += 1; // handled globally; tolerate positionally too
            }
            s if !s.starts_with('-') && expr.is_none() => {
                expr = Some(s);
                i += 1;
            }
            other => {
                output::print_error(&format!("Unknown argument: {other}"));
                print_usage();
                return ExitCode::Usage;
            }
        }
    }

    let Some(expr) = expr else {
        print_usage();
        return ExitCode::Usage;
    };

    let auth = match parse_auth(auth_kvs.unwrap_or("")) {
        Ok(a) => a,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };
    let row: Option<serde_json::Value> = match row_json {
        None => None,
        Some(s) => match serde_json::from_str(s) {
            Ok(v) => Some(v),
            Err(e) => {
                output::print_error(&format!("--row is not valid JSON: {e}"));
                return ExitCode::Usage;
            }
        },
    };
    let input: Option<serde_json::Value> = match input_json {
        None => None,
        Some(s) => match serde_json::from_str(s) {
            Ok(v) => Some(v),
            Err(e) => {
                output::print_error(&format!("--input is not valid JSON: {e}"));
                return ExitCode::Usage;
            }
        },
    };

    let result = pylon_policy::evaluate_expression(expr, &auth, row.as_ref(), input.as_ref());

    match result {
        PolicyResult::Allowed => {
            if json_mode {
                println!(
                    "{}",
                    serde_json::json!({
                        "result": "allow",
                        "expr": expr,
                        "auth": auth,
                        "row": row,
                    })
                );
            } else {
                println!("ALLOW");
                println!("  expr: {expr}");
                println!("  auth: {}", summarize_auth(&auth));
            }
            ExitCode::Ok
        }
        PolicyResult::Denied { reason, .. } => {
            if json_mode {
                println!(
                    "{}",
                    serde_json::json!({
                        "result": "deny",
                        "reason": reason,
                        "expr": expr,
                        "auth": auth,
                        "row": row,
                    })
                );
            } else {
                println!("DENY");
                println!("  expr:   {expr}");
                println!("  auth:   {}", summarize_auth(&auth));
                println!("  reason: {reason}");
            }
            // Distinct from usage errors so scripts can branch:
            // allow=0, deny=1, bad invocation=64.
            ExitCode::Error
        }
    }
}

/// Parse `--auth userId=u1,isAdmin=true,tenantId=t1,roles=admin|editor`
/// into an AuthContext. Unlisted fields keep anonymous defaults, so an
/// empty string is a faithful anonymous caller.
fn parse_auth(kvs: &str) -> Result<AuthContext, String> {
    let mut auth = AuthContext::anonymous();
    for pair in kvs.split(',').filter(|p| !p.trim().is_empty()) {
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| format!("--auth entries are key=value, got {pair:?}"))?;
        match k.trim() {
            "userId" | "user_id" => auth.user_id = Some(v.trim().to_string()),
            "isAdmin" | "is_admin" => {
                auth.is_admin = parse_bool(v).ok_or_else(|| bad_bool("isAdmin", v))?
            }
            "isGuest" | "is_guest" => {
                auth.is_guest = parse_bool(v).ok_or_else(|| bad_bool("isGuest", v))?
            }
            "tenantId" | "tenant_id" => auth.tenant_id = Some(v.trim().to_string()),
            "roles" => {
                auth.roles = v
                    .split('|')
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty())
                    .collect()
            }
            other => {
                return Err(format!(
                    "Unknown --auth key {other:?}. Known: userId, isAdmin, isGuest, tenantId, roles (pipe-separated)."
                ));
            }
        }
    }
    Ok(auth)
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn bad_bool(key: &str, v: &str) -> String {
    format!("--auth {key} must be true/false, got {v:?}")
}

fn summarize_auth(auth: &AuthContext) -> String {
    format!(
        "userId={} isAdmin={} isGuest={} tenantId={} roles=[{}]",
        auth.user_id.as_deref().unwrap_or("null"),
        auth.is_admin,
        auth.is_guest,
        auth.tenant_id.as_deref().unwrap_or("null"),
        auth.roles.join("|"),
    )
}

fn print_usage() {
    println!(
        "Usage: pylon policy test <expr> [--auth k=v,...] [--row <json>] [--input <json>] [--json]"
    );
    println!();
    println!("  Dry-run a policy expression with the production evaluator.");
    println!("  Exit codes: 0 = allowed, 1 = denied, 64 = usage error.");
    println!();
    println!("  --auth   userId=u1,isAdmin=false,isGuest=false,tenantId=t1,roles=admin|editor");
    println!("  --row    the row's JSON (binds data.*)");
    println!("  --input  the incoming write's JSON (binds input.*)");
    println!();
    println!("  Example:");
    println!("    pylon policy test 'auth.userId == data.ownerId' \\");
    println!("      --auth userId=u1 --row '{{\"ownerId\":\"u1\"}}'");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_auth_full_shape() {
        let a = parse_auth("userId=u1,isAdmin=true,isGuest=false,tenantId=t9,roles=admin|editor")
            .unwrap();
        assert_eq!(a.user_id.as_deref(), Some("u1"));
        assert!(a.is_admin);
        assert!(!a.is_guest);
        assert_eq!(a.tenant_id.as_deref(), Some("t9"));
        assert_eq!(a.roles, vec!["admin", "editor"]);
    }

    #[test]
    fn parse_auth_empty_is_anonymous() {
        let a = parse_auth("").unwrap();
        assert!(a.user_id.is_none());
        assert!(!a.is_admin);
        assert!(a.roles.is_empty());
    }

    #[test]
    fn parse_auth_rejects_unknown_keys_and_bad_bools() {
        assert!(parse_auth("email=x@y.com").is_err());
        assert!(parse_auth("isAdmin=yes").is_err());
    }

    #[test]
    fn evaluator_round_trip_allow_and_deny() {
        let auth = parse_auth("userId=u1").unwrap();
        let row = serde_json::json!({"ownerId": "u1"});
        assert!(matches!(
            pylon_policy::evaluate_expression(
                "auth.userId == data.ownerId",
                &auth,
                Some(&row),
                None
            ),
            pylon_policy::PolicyResult::Allowed
        ));
        let other = serde_json::json!({"ownerId": "u2"});
        assert!(matches!(
            pylon_policy::evaluate_expression(
                "auth.userId == data.ownerId",
                &auth,
                Some(&other),
                None
            ),
            pylon_policy::PolicyResult::Denied { .. }
        ));
    }
}
