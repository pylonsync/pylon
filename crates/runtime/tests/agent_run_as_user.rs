//! `ctx.agents.run` end-to-end: a real server, the real Bun runtime, and
//! a stub Anthropic provider on a local port.
//!
//!   - an elevated (webhook-style) action runs an agent as a named user;
//!     the run belongs to that user, its tools receive the server-set
//!     context, and the context is stored on the run
//!   - a public action that did not elevate is refused
//!   - the target must be an `agent()`
//!   - an agent declared `auth: "admin"` refuses a non-admin identity
//!
//! Skipped (with a message) when `bun` is not on PATH.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::AppManifest;
use pylon_runtime::Runtime;

fn bun_available() -> bool {
    std::process::Command::new("bun")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    without_verbatim_prefix(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap(),
    )
}

/// `canonicalize` returns a `\\?\` verbatim path on Windows, which bun
/// cannot resolve as a module path. Drop the prefix; other platforms are
/// unchanged.
fn without_verbatim_prefix(p: PathBuf) -> PathBuf {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

/// `abs` as a path relative to the current directory. The runtime joins
/// `PYLON_FUNCTIONS_DIR` onto the process cwd, so it must be relative.
fn relative_to_cwd(abs: &Path) -> String {
    let cwd = std::env::current_dir().unwrap();
    let ups = cwd.components().count() - 1;
    let mut rel = "../".repeat(ups);
    rel.push_str(abs.to_str().unwrap().trim_start_matches('/'));
    rel
}

/// One SSE response per request: the first asks for the `whoami` tool,
/// every later one answers with text.
fn stub_anthropic() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for sock in listener.incoming() {
            let Ok(mut sock) = sock else { continue };
            let n = requests.fetch_add(1, Ordering::SeqCst);
            // Read the head, then exactly Content-Length body bytes.
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            let events = if n == 0 {
                concat!(
                    "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":1}}}\n\n",
                    "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"tu_1\",\"name\":\"whoami\",\"input\":{}}}\n\n",
                    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
                    "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
                    "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":1}}\n\n",
                    "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
                )
            } else {
                concat!(
                    "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":1}}}\n\n",
                    "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"done\"}}\n\n",
                    "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
                    "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
                    "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
                )
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                events.len()
            );
            let _ = sock.write_all(head.as_bytes());
            let _ = sock.write_all(events.as_bytes());
        }
    });
    base
}

fn functions_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pylon-agent-run-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let fns = dir.join("functions");
    std::fs::create_dir_all(&fns).unwrap();
    let src = repo_root().join("packages/functions/src");
    let agent_ts = src.join("agent.ts");
    let define_ts = src.join("define.ts");
    let validators_ts = src.join("validators.ts");
    let files = [
        (
            "helper",
            format!(
                r#"import {{ agent }} from "{agent}";
export default agent({{
  model: "m",
  tools: {{
    whoami: {{
      description: "Report who the run is for",
      handler: async (ctx, _input, run) => ({{
        authUser: ctx.auth.userId,
        runUser: run.userId,
        context: run.context,
      }}),
    }},
  }},
}});
"#,
                agent = agent_ts.display()
            ),
        ),
        (
            "adminOnly",
            format!(
                r#"import {{ agent }} from "{agent}";
export default agent({{ auth: "admin", model: "m" }});
"#,
                agent = agent_ts.display()
            ),
        ),
        (
            "plainAction",
            format!(
                r#"import {{ action }} from "{define}";
export default action({{ auth: "public", handler: async () => "plain" }});
"#,
                define = define_ts.display()
            ),
        ),
        (
            "kickoff",
            format!(
                r#"import {{ action }} from "{define}";
import {{ v }} from "{validators}";
export default action({{
  auth: "public",
  args: {{ agent: v.string(), elevate: v.boolean() }},
  handler: async (ctx, args) => {{
    if (args.elevate) await ctx.auth.elevate({{ admin: true, reason: "verified webhook" }});
    try {{
      return await ctx.agents.run(args.agent, {{
        input: "hello",
        as: {{ userId: "owner-1" }},
        context: {{ conversationId: "conv-9" }},
      }});
    }} catch (err) {{
      return {{ error: err.code ?? String(err) }};
    }}
  }},
}});
"#,
                define = define_ts.display(),
                validators = validators_ts.display()
            ),
        ),
    ];
    for (name, src) in files {
        std::fs::write(fns.join(format!("{name}.ts")), src).unwrap();
    }
    fns
}

fn manifest() -> AppManifest {
    let field = |name: &str, ty: &str, optional: bool| serde_json::json!({ "name": name, "type": ty, "optional": optional, "unique": false });
    serde_json::from_value(serde_json::json!({
        "manifest_version": 1,
        "name": "agent-run",
        "version": "0.1.0",
        "entities": [
            {
                "name": "AgentRun",
                "fields": [
                    field("agent", "string", false),
                    field("status", "string", false),
                    field("userId", "string", true),
                    field("title", "string", true),
                    field("streamId", "string", true),
                    field("error", "string", true),
                    field("pendingInput", "json", true),
                    field("cancelRequested", "bool", true),
                    field("steps", "int", true),
                    field("context", "json", true),
                    field("createdAt", "datetime", false),
                    field("updatedAt", "datetime", false),
                ],
                "indexes": [],
                "relations": []
            },
            {
                "name": "AgentMessage",
                "fields": [
                    field("runId", "string", false),
                    field("userId", "string", true),
                    field("seq", "int", false),
                    field("role", "string", false),
                    field("content", "json", true),
                    field("createdAt", "datetime", false),
                ],
                "indexes": [],
                "relations": []
            }
        ],
        "routes": [],
        "queries": [],
        "actions": [],
        "policies": []
    }))
    .expect("manifest parses")
}

fn post(port: u16, path: &str, body: &serde_json::Value) -> (u16, serde_json::Value) {
    let body = body.to_string();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    stream.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    let status: u16 = raw
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    let json = serde_json::from_str(&body).unwrap_or(serde_json::Value::String(body));
    (status, json)
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn server_code_runs_an_agent_as_a_named_user() {
    if !bun_available() {
        eprintln!("skipped: bun is not on PATH");
        return;
    }
    let provider = stub_anthropic();
    let fns = functions_dir();
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_FUNCTIONS_DIR", relative_to_cwd(&fns));
        std::env::set_var(
            "PYLON_FUNCTIONS_RUNTIME",
            repo_root().join("packages/functions/src/runtime.ts"),
        );
        std::env::set_var("PYLON_FN_POOL_SIZE", "1");
        std::env::set_var("PYLON_LLM_PROVIDER", "anthropic");
        std::env::set_var("ANTHROPIC_API_KEY", "test-key");
        std::env::set_var("PYLON_LLM_BASE_URL", &provider);
    }

    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    let port = free_port();
    let server_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(server_rt, port);
    });

    // Wait until the functions runtime serves calls.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            let (status, body) = post(port, "/api/fn/plainAction", &serde_json::json!({}));
            if status == 200 && body == "plain" {
                break;
            }
        }
        assert!(Instant::now() < deadline, "functions never came up");
        std::thread::sleep(Duration::from_millis(200));
    }

    // A public action a client calls cannot pick an identity.
    let (status, body) = post(
        port,
        "/api/fn/kickoff",
        &serde_json::json!({ "agent": "helper", "elevate": false }),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["error"], "AGENT_RUN_FORBIDDEN", "{body}");

    // Only agent() functions can be run this way.
    let (_, body) = post(
        port,
        "/api/fn/kickoff",
        &serde_json::json!({ "agent": "plainAction", "elevate": true }),
    );
    assert_eq!(body["error"], "AGENT_NOT_FOUND", "{body}");

    // The agent's own auth mode still applies to the identity.
    let (_, body) = post(
        port,
        "/api/fn/kickoff",
        &serde_json::json!({ "agent": "adminOnly", "elevate": true }),
    );
    assert_eq!(body["error"], "FORBIDDEN", "{body}");
    assert!(
        rt.list("AgentRun").unwrap().is_empty(),
        "no refused call may create a run"
    );

    // A verified webhook (elevated) runs the agent as owner-1.
    let (status, body) = post(
        port,
        "/api/fn/kickoff",
        &serde_json::json!({ "agent": "helper", "elevate": true }),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["text"], "done", "{body}");
    let run_id = body["runId"].as_str().expect("runId").to_string();

    let runs = rt.list("AgentRun").unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["id"], run_id);
    assert_eq!(runs[0]["userId"], "owner-1");
    assert_eq!(runs[0]["status"], "completed");
    assert_eq!(runs[0]["context"]["conversationId"], "conv-9");

    // The tool ran as owner-1 and received the server-set context.
    let messages = rt.list("AgentMessage").unwrap();
    let tool_result = messages
        .iter()
        .filter(|m| m["role"] == "user")
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .find(|b| b["type"] == "tool_result")
        .expect("a tool_result was persisted");
    let seen: serde_json::Value =
        serde_json::from_str(tool_result["content"].as_str().unwrap()).unwrap();
    assert_eq!(seen["authUser"], "owner-1");
    assert_eq!(seen["runUser"], "owner-1");
    assert_eq!(seen["context"]["conversationId"], "conv-9");
    assert!(messages.iter().all(|m| m["userId"] == "owner-1"));
}
