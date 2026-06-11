//! `pylon fn <name> [key=value ...]` — call any Pylon Cloud function
//! by name with structured args from the shell.
//!
//! Useful for one-off admin / diagnostic functions (inspectProjectGitConfig,
//! findProjectsBySlug, etc.) without writing a curl invocation by hand.
//! Wraps POST /api/fn/<name> with auth + JSON-shaped args.
//!
//! Arg syntax: `key=value` pairs after the function name.
//! - `key=value` — string by default
//! - `key=42` / `key=3.14` — parsed as number
//! - `key=true` / `key=false` — parsed as bool
//! - `key=null` — parsed as null
//! - `key=@path/to/file.json` — read JSON body from file
//! - `key={"foo":"bar"}` or `key=[1,2,3]` — inlined JSON literal
//!
//! Example:
//!   pylon fn inspectProjectGitConfig projectSlug=chat-api
//!   pylon fn createOrgInvite orgId=org_123 email=alice@example.com role=admin
//!   pylon fn upsertEntity payload=@row.json

use pylon_kernel::ExitCode;
use serde_json::Value;

use crate::cloud_client::{post_json, require_credentials};
use crate::output;

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let creds = match require_credentials() {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&e);
            eprintln!("  Run: pylon login");
            return ExitCode::Usage;
        }
    };

    // First positional after "fn" is the function name; remaining are kv args.
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|s| s.as_str())
        .collect();
    let fn_name = match positional.iter().position(|a| *a == "fn") {
        Some(idx) => match positional.get(idx + 1) {
            Some(name) => *name,
            None => {
                output::print_error("Usage: pylon fn <function-name> [key=value ...]");
                return ExitCode::Usage;
            }
        },
        None => {
            output::print_error("Usage: pylon fn <function-name> [key=value ...]");
            return ExitCode::Usage;
        }
    };
    let kv_args: Vec<&str> = positional
        .into_iter()
        .skip_while(|a| *a != "fn")
        .skip(2) // "fn" + <name>
        .collect();

    let body = match build_args_json(&kv_args) {
        Ok(v) => v,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };

    let path = format!("/api/fn/{fn_name}");
    let result: Result<Value, String> = post_json(&creds, &path, &body);
    match result {
        Ok(v) => {
            if json_mode {
                println!("{}", serde_json::to_string(&v).unwrap_or_default());
            } else {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            ExitCode::Ok
        }
        Err(e) => {
            output::print_error(&format!("Cloud call failed: {e}"));
            ExitCode::Error
        }
    }
}

/// Parse `key=value` pairs into a JSON object. Values get type-coerced
/// per the rules in the module docstring.
fn build_args_json(kv_args: &[&str]) -> Result<Value, String> {
    let mut obj = serde_json::Map::new();
    for arg in kv_args {
        let (key, raw) = match arg.split_once('=') {
            Some(parts) => parts,
            None => {
                return Err(format!("argument `{arg}` is not in key=value form"));
            }
        };
        if key.is_empty() {
            return Err(format!("argument `{arg}` has empty key"));
        }
        let value = coerce_value(raw)?;
        obj.insert(key.to_string(), value);
    }
    Ok(Value::Object(obj))
}

fn coerce_value(raw: &str) -> Result<Value, String> {
    // @path → load file as JSON.
    if let Some(path) = raw.strip_prefix('@') {
        let bytes = std::fs::read(path).map_err(|e| format!("failed to read {path}: {e}"))?;
        return serde_json::from_slice::<Value>(&bytes)
            .map_err(|e| format!("`{path}` is not valid JSON: {e}"));
    }
    // Inlined JSON literals: { } or [ ] prefix.
    let trimmed = raw.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<Value>(raw) {
            return Ok(v);
        }
        // Fall through to treat as string if it didn't parse — better
        // than rejecting outright; user might pass {raw-not-json} as
        // a literal string they want preserved.
    }
    // Bool / null / number / string fallback.
    match raw {
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        "null" => Ok(Value::Null),
        _ => {
            if let Ok(i) = raw.parse::<i64>() {
                return Ok(Value::from(i));
            }
            if let Ok(f) = raw.parse::<f64>() {
                if f.is_finite() {
                    return Ok(Value::from(f));
                }
            }
            Ok(Value::String(raw.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    // `3.14` is arbitrary float test data for the number-coercion path, not an
    // attempt to spell π — silence clippy::approx_constant for this fixture.
    #[allow(clippy::approx_constant)]
    fn coerces_strings_numbers_bools() {
        assert_eq!(coerce_value("hello").unwrap(), json!("hello"));
        assert_eq!(coerce_value("42").unwrap(), json!(42));
        assert_eq!(coerce_value("3.14").unwrap(), json!(3.14));
        assert_eq!(coerce_value("true").unwrap(), json!(true));
        assert_eq!(coerce_value("false").unwrap(), json!(false));
        assert_eq!(coerce_value("null").unwrap(), json!(null));
    }

    #[test]
    fn coerces_inline_json_literals() {
        assert_eq!(
            coerce_value(r#"{"foo":"bar"}"#).unwrap(),
            json!({ "foo": "bar" })
        );
        assert_eq!(coerce_value("[1,2,3]").unwrap(), json!([1, 2, 3]));
    }

    #[test]
    fn builds_args_object_from_pairs() {
        let args = vec!["projectSlug=chat-api", "limit=10", "force=true"];
        let v = build_args_json(&args).unwrap();
        assert_eq!(
            v,
            json!({ "projectSlug": "chat-api", "limit": 10, "force": true })
        );
    }

    #[test]
    fn rejects_non_kv_arg() {
        let err = build_args_json(&["not-a-pair"]).unwrap_err();
        assert!(err.contains("not in key=value form"));
    }

    #[test]
    fn rejects_empty_key() {
        let err = build_args_json(&["=foo"]).unwrap_err();
        assert!(err.contains("empty key"));
    }
}
