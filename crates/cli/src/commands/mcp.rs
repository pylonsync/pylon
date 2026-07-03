//! `pylon mcp` — expose a running Pylon app to agents over MCP.
//!
//! Studio is the human's window into a live app; this is the agent's.
//! Any MCP-speaking agent (Claude Code, Codex, Cursor, …) can connect
//! and get schema introspection, data reads, function calls, policy
//! dry-runs, and route verification — without shelling out or knowing
//! Pylon's HTTP surface.
//!
//! Transport: MCP stdio — newline-delimited JSON-RPC 2.0 on
//! stdin/stdout (logs go to stderr, never stdout: a stray print there
//! corrupts the protocol stream).
//!
//! Register with an agent, e.g. Claude Code:
//!
//! ```text
//! claude mcp add pylon -- pylon mcp --url http://localhost:4321
//! ```
//!
//! Tools:
//!   pylon_schema       entities/fields/policies/routes/functions from the
//!                      project manifest (cwd's pylon.manifest.json)
//!   pylon_list         rows from an entity (policy-enforced by the app)
//!   pylon_get          one row by id (policy-enforced by the app)
//!   pylon_call         call a function by name with JSON args
//!   pylon_policy_test  dry-run a policy expression (local evaluator —
//!                      the same code enforcement runs)
//!   pylon_verify       run `pylon verify`'s checks against the app
//!
//! Data access rides the app's HTTP API with the caller's `--token`
//! (or anonymously without one), so row policies apply exactly as they
//! would to any client — this surface grants NO extra authority.

use std::io::{BufRead, Write};

use pylon_kernel::ExitCode;
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2024-11-05";

pub struct McpConfig {
    pub base_url: String,
    pub token: Option<String>,
}

pub fn run(args: &[String], _json_mode: bool) -> ExitCode {
    let base_url = flag_value(args, "--url").unwrap_or_else(|| "http://localhost:4321".to_string());
    let token = flag_value(args, "--token");
    let cfg = McpConfig {
        base_url: base_url.trim_end_matches('/').to_string(),
        token,
    };

    eprintln!(
        "[pylon mcp] serving MCP on stdio → {} (token: {})",
        cfg.base_url,
        if cfg.token.is_some() {
            "yes"
        } else {
            "none (anonymous)"
        }
    );

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break, // stdin closed — host went away
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle_line(&cfg, &line) {
            let mut out = stdout.lock();
            let _ = writeln!(out, "{response}");
            let _ = out.flush();
        }
    }
    ExitCode::Ok
}

/// Handle one JSON-RPC line. Returns the response to write, or None
/// for notifications (which get no response by spec).
pub fn handle_line(cfg: &McpConfig, line: &str) -> Option<String> {
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            // Parse error → id is unknowable; JSON-RPC says respond with null id.
            return Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("parse error: {e}")}})
                    .to_string(),
            );
        }
    };
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let id = msg.get("id").cloned();

    // Notifications (no id) get handled silently.
    if id.is_none() {
        return None;
    }
    let id = id.unwrap();

    let result: Result<Value, (i64, String)> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "pylon",
                "version": env!("CARGO_PKG_VERSION"),
            },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(dispatch_tool(cfg, name, &args))
        }
        other => Err((-32601, format!("method not found: {other}"))),
    };

    let response = match result {
        Ok(value) => json!({"jsonrpc":"2.0","id":id,"result":value}),
        Err((code, message)) => {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
        }
    };
    Some(response.to_string())
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "pylon_schema",
            "description": "The app's schema: entities with fields, policies, routes, and functions. Read from the project manifest in the working directory.",
            "inputSchema": {"type":"object","properties":{},"additionalProperties":false}
        },
        {
            "name": "pylon_list",
            "description": "List rows from an entity via the app's HTTP API. Row policies apply — you see exactly what a client with this token sees.",
            "inputSchema": {"type":"object","properties":{
                "entity": {"type":"string","description":"Entity name, e.g. \"Doc\""},
                "limit": {"type":"number","description":"Max rows (default 20)"}
            },"required":["entity"],"additionalProperties":false}
        },
        {
            "name": "pylon_get",
            "description": "Fetch one entity row by id via the app's HTTP API (policy-enforced).",
            "inputSchema": {"type":"object","properties":{
                "entity": {"type":"string"},
                "id": {"type":"string"}
            },"required":["entity","id"],"additionalProperties":false}
        },
        {
            "name": "pylon_call",
            "description": "Call a Pylon function by name with JSON arguments (POST /api/fn/<name>). Auth is the server's token if one was provided.",
            "inputSchema": {"type":"object","properties":{
                "name": {"type":"string","description":"Function name, e.g. \"seedPad\""},
                "args": {"type":"object","description":"JSON arguments object (default {})"}
            },"required":["name"],"additionalProperties":false}
        },
        {
            "name": "pylon_policy_test",
            "description": "Dry-run a policy expression with the production evaluator. Returns allow/deny with the evaluator's reason. auth keys: userId, isAdmin, isGuest, tenantId, roles (array).",
            "inputSchema": {"type":"object","properties":{
                "expr": {"type":"string","description":"Policy expression, e.g. \"auth.userId == data.ownerId\""},
                "auth": {"type":"object","description":"Auth context: {userId, isAdmin, isGuest, tenantId, roles}"},
                "row": {"type":"object","description":"Row JSON binding data.*"},
                "input": {"type":"object","description":"Incoming write JSON binding input.*"}
            },"required":["expr"],"additionalProperties":false}
        },
        {
            "name": "pylon_verify",
            "description": "Verify the app serves: /health, every static manifest route, and every referenced JS/CSS asset. Returns the per-check report.",
            "inputSchema": {"type":"object","properties":{},"additionalProperties":false}
        }
    ])
}

/// Run a tool and wrap the outcome in MCP content. Tool-level failures
/// are `isError: true` results (the agent can read and react), not
/// protocol errors.
fn dispatch_tool(cfg: &McpConfig, name: &str, args: &Value) -> Value {
    let outcome: Result<Value, String> = match name {
        "pylon_schema" => tool_schema(),
        "pylon_list" => tool_list(cfg, args),
        "pylon_get" => tool_get(cfg, args),
        "pylon_call" => tool_call(cfg, args),
        "pylon_policy_test" => tool_policy_test(args),
        "pylon_verify" => tool_verify(cfg),
        other => Err(format!("unknown tool: {other}")),
    };
    match outcome {
        Ok(v) => json!({
            "content": [{"type":"text","text": serde_json::to_string_pretty(&v).unwrap_or_default()}]
        }),
        Err(e) => json!({
            "content": [{"type":"text","text": e}],
            "isError": true
        }),
    }
}

fn tool_schema() -> Result<Value, String> {
    let raw = std::fs::read_to_string("pylon.manifest.json").map_err(|e| {
        format!(
            "no pylon.manifest.json in the working directory ({e}) — \
             run `pylon mcp` from the app's root, or run `pylon codegen` first"
        )
    })?;
    let manifest: Value =
        serde_json::from_str(&raw).map_err(|e| format!("manifest parse error: {e}"))?;
    // The manifest is already the agent-legible projection of app.ts —
    // pass through the sections that matter for reasoning about the app.
    Ok(json!({
        "entities": manifest.get("entities"),
        "policies": manifest.get("policies"),
        "routes": manifest.get("routes"),
        "functions": manifest.get("functions"),
        "auth": manifest.get("auth"),
    }))
}

fn tool_list(cfg: &McpConfig, args: &Value) -> Result<Value, String> {
    let entity = require_str(args, "entity")?;
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20);
    http_get_json(cfg, &format!("/api/entities/{entity}?limit={limit}"))
}

fn tool_get(cfg: &McpConfig, args: &Value) -> Result<Value, String> {
    let entity = require_str(args, "entity")?;
    let id = require_str(args, "id")?;
    http_get_json(cfg, &format!("/api/entities/{entity}/{id}"))
}

fn tool_call(cfg: &McpConfig, args: &Value) -> Result<Value, String> {
    let name = require_str(args, "name")?;
    let fn_args = args.get("args").cloned().unwrap_or(json!({}));
    let url = format!("{}/api/fn/{name}", cfg.base_url);
    let mut req = ureq::post(&url).set("content-type", "application/json");
    if let Some(token) = &cfg.token {
        req = req.set("authorization", &format!("Bearer {token}"));
    }
    match req.send_string(&fn_args.to_string()) {
        Ok(resp) => resp
            .into_json::<Value>()
            .map_err(|e| format!("response was not JSON: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            // The error body is the useful part (code + message from the
            // app) — return it as data, tagged with the HTTP status.
            Ok(json!({"http_status": code, "error_body": parse_or_string(&body)}))
        }
        Err(e) => Err(format!("request failed: {e}")),
    }
}

fn tool_policy_test(args: &Value) -> Result<Value, String> {
    let expr = require_str(args, "expr")?;
    let mut auth = pylon_auth::AuthContext::anonymous();
    if let Some(a) = args.get("auth") {
        if let Some(uid) = a.get("userId").and_then(Value::as_str) {
            auth.user_id = Some(uid.to_string());
        }
        if let Some(b) = a.get("isAdmin").and_then(Value::as_bool) {
            auth.is_admin = b;
        }
        if let Some(b) = a.get("isGuest").and_then(Value::as_bool) {
            auth.is_guest = b;
        }
        if let Some(t) = a.get("tenantId").and_then(Value::as_str) {
            auth.tenant_id = Some(t.to_string());
        }
        if let Some(roles) = a.get("roles").and_then(Value::as_array) {
            auth.roles = roles
                .iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect();
        }
    }
    let row = args.get("row").filter(|v| !v.is_null()).cloned();
    let input = args.get("input").filter(|v| !v.is_null()).cloned();
    match pylon_policy::evaluate_expression(expr, &auth, row.as_ref(), input.as_ref()) {
        pylon_policy::PolicyResult::Allowed => Ok(json!({"result": "allow"})),
        pylon_policy::PolicyResult::Denied { reason, .. } => {
            Ok(json!({"result": "deny", "reason": reason}))
        }
    }
}

fn tool_verify(cfg: &McpConfig) -> Result<Value, String> {
    let route_paths: Vec<String> = match crate::manifest::load_manifest("pylon.manifest.json") {
        Ok(m) => m.routes.iter().map(|r| r.path.clone()).collect(),
        Err(_) => vec!["/".to_string()],
    };
    let report = crate::commands::verify::verify_target(&cfg.base_url, &route_paths);
    serde_json::to_value(&report).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].clone())
        .or_else(|| {
            let prefix = format!("{flag}=");
            args.iter()
                .find(|a| a.starts_with(&prefix))
                .map(|a| a[prefix.len()..].to_string())
        })
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing required argument: {key}"))
}

fn http_get_json(cfg: &McpConfig, path: &str) -> Result<Value, String> {
    let url = format!("{}{path}", cfg.base_url);
    let mut req = ureq::get(&url);
    if let Some(token) = &cfg.token {
        req = req.set("authorization", &format!("Bearer {token}"));
    }
    match req.call() {
        Ok(resp) => resp
            .into_json::<Value>()
            .map_err(|e| format!("response was not JSON: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Ok(json!({"http_status": code, "error_body": parse_or_string(&body)}))
        }
        Err(e) => Err(format!("request failed: {e}")),
    }
}

fn parse_or_string(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> McpConfig {
        McpConfig {
            base_url: "http://127.0.0.1:1".to_string(),
            token: None,
        }
    }

    fn rpc(cfg: &McpConfig, body: Value) -> Value {
        let line = body.to_string();
        let resp = handle_line(cfg, &line).expect("expected a response");
        serde_json::from_str(&resp).unwrap()
    }

    #[test]
    fn initialize_advertises_tools() {
        let r = rpc(
            &cfg(),
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        );
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(r["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn notifications_get_no_response() {
        assert!(handle_line(
            &cfg(),
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string()
        )
        .is_none());
    }

    #[test]
    fn tools_list_names_every_tool() {
        let r = rpc(
            &cfg(),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        );
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for expected in [
            "pylon_schema",
            "pylon_list",
            "pylon_get",
            "pylon_call",
            "pylon_policy_test",
            "pylon_verify",
        ] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }
    }

    #[test]
    fn policy_test_tool_round_trips_without_network() {
        let r = rpc(
            &cfg(),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
                "name":"pylon_policy_test",
                "arguments":{"expr":"auth.userId == data.ownerId",
                              "auth":{"userId":"u1"},
                              "row":{"ownerId":"u1"}}}}),
        );
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"allow\""), "got: {text}");

        let r = rpc(
            &cfg(),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
                "name":"pylon_policy_test",
                "arguments":{"expr":"auth.isAdmin"}}}),
        );
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"deny\""), "got: {text}");
    }

    #[test]
    fn unknown_tool_is_tool_error_not_protocol_error() {
        let r = rpc(
            &cfg(),
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nope"}}),
        );
        assert_eq!(r["result"]["isError"], true);
    }

    #[test]
    fn parse_error_returns_null_id_error() {
        let resp = handle_line(&cfg(), "{not json").unwrap();
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["error"]["code"], -32700);
        assert!(v["id"].is_null());
    }

    #[test]
    fn unknown_method_errors() {
        let r = rpc(&cfg(), json!({"jsonrpc":"2.0","id":6,"method":"bogus"}));
        assert_eq!(r["error"]["code"], -32601);
    }
}
