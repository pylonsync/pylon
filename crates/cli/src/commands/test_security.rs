//! `pylon test:security` — adversarial probe against a running app.
//! Enumerates every function and entity, fires anonymous requests at
//! each, and reports anything that answers without an auth gate.
//!
//! Catches the canonical "I made a function and forgot to mark it
//! `auth: \"user\"` (or shipped a policy that allows anonymous
//! reads)" class.
//!
//! Usage:
//!   pylon test:security                  # probes http://localhost:4321
//!   pylon test:security --target https://api.example.com
//!   pylon test:security --admin-token <t>   # or PYLON_ADMIN_TOKEN
//!   pylon test:security --writes         # also probe anonymous inserts
//!   pylon test:security --public-entities Post,Board   # meant to be readable
//!   pylon test:security --json --strict
//!
//! Where the function list comes from, best first:
//!   1. `GET /api/fn` on the target. Admin-only, so it needs the app's
//!      admin token (or a dev server). It lists every function with its
//!      declared `auth` and `internal` flags, mutations included.
//!   2. Otherwise the manifest's queries and actions, plus the file
//!      names in `functions/` (one function per file), so mutations are
//!      probed too. Declared auth is unknown on this path.
//!
//! Probes never run a function. Each function is called with a body
//! that is not JSON: the router checks auth before it parses the body,
//! so a gated function answers 401/403 and a reachable one answers
//! 400 INVALID_JSON without its handler running. That makes the probe
//! safe against production.
//!
//! Entity inserts are different: the body is parsed before the policy
//! runs, so the only probe that tests the insert policy is a real
//! insert. It runs against localhost, or anywhere with `--writes`.
//!
//! Levels:
//!   - finding: a function not declared public answered past the auth
//!     gate, an entity not in `--public-entities` returned rows to an
//!     anonymous list, or an entity accepted an anonymous insert.
//!     `--strict` exits non-zero on these.
//!   - review: a function declared `auth: "public"` or `"guest"`, one
//!     whose declared auth is unknown and answered past the gate, or an
//!     entity in `--public-entities` that returned rows. Confirm each is
//!     meant to be open.
//!   - info: an entity answered an anonymous list with zero rows. Its
//!     read policy filtered every row or the table is empty.
//!
//! What it does NOT do:
//!   - Cross-tenant probing (needs test users + data). Use the policy
//!     linter (`pylon lint`) for static cross-tenant checks and ship
//!     integration tests for the dynamic ones.
//!   - Replay-based attacks (timing, parameter tampering on signed
//!     URLs, etc.). Handled separately by the auth-flow integration
//!     tests in the framework.

use std::path::Path;
use std::time::Duration;

use pylon_kernel::{AppManifest, ExitCode};
use serde::{Deserialize, Serialize};
use ureq;

use crate::output;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Level {
    Finding,
    Review,
    Info,
}

#[derive(Debug, Clone, Serialize)]
struct Finding {
    level: Level,
    target: String,
    kind: &'static str,
    status: u16,
    expected: &'static str,
    detail: String,
}

#[derive(Debug, Serialize)]
struct JsonReport<'a> {
    target_base_url: &'a str,
    functions_source: &'static str,
    probes: usize,
    skipped: &'a [String],
    findings: &'a [Finding],
}

/// A function to probe. `auth` and `kind` are known only when the list
/// came from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FnTarget {
    name: String,
    kind: Option<String>,
    auth: Option<String>,
}

/// One entry of `GET /api/fn` (the runtime's `FnDef`).
#[derive(Debug, Deserialize)]
struct ListedFn {
    name: String,
    #[serde(default)]
    fn_type: Option<String>,
    #[serde(default)]
    internal: bool,
    #[serde(default)]
    auth: Option<String>,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let target = pick_target(args);
    let strict = args.iter().any(|a| a == "--strict");
    let admin_token = flag_value(args, "--admin-token")
        .or_else(|| std::env::var("PYLON_ADMIN_TOKEN").ok())
        .filter(|t| !t.trim().is_empty());
    let writes = args.iter().any(|a| a == "--writes") || is_local(&target);
    let origin = origin_of(&target);
    let public_entities: Vec<String> = flag_value(args, "--public-entities")
        .map(|v| {
            v.split(',')
                .map(|e| e.trim().to_string())
                .filter(|e| !e.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let manifest = match load_manifest() {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };

    // The 5s timeout is per-probe — generous enough to absorb a
    // cold-start function but short enough that a probe of a thousand
    // functions doesn't run for hours. Connect timeout shorter so
    // unreachable targets fail fast.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout_read(Duration::from_secs(5))
        .user_agent("pylon-test-security/0.2")
        .build();

    let mut findings = Vec::new();
    let mut skipped = Vec::new();
    let mut probes = 0;

    let (functions, functions_source) =
        match list_functions(&agent, &target, admin_token.as_deref()) {
            Some(listed) => (listed, "server"),
            None => {
                let dir_names = function_file_names(Path::new("functions"));
                (functions_from_manifest(&manifest, &dir_names), "manifest")
            }
        };

    // Probe 1 — every function via /api/fn/<name>, anonymous, with a
    // body that is not JSON (see the module docs).
    for f in &functions {
        probes += 1;
        let url = format!("{target}/api/fn/{}", f.name);
        let (status, body) = match probe_anon_post(&agent, &url, &origin, "{") {
            ProbeResult::Status(s, b) => (s, b),
            ProbeResult::ConnectionError(e) => {
                output::print_error(&format!("Could not reach {url}: {e}"));
                return ExitCode::Unavailable;
            }
        };
        match classify_fn(status, &body) {
            FnProbe::Gated => {}
            FnProbe::Inconclusive => {
                skipped.push(format!("{} answered {status}; not checked", f.name))
            }
            FnProbe::Reachable => findings.push(fn_finding(f, &url, status)),
        }
    }

    // Probe 2 — every entity via /api/entities/<entity>.
    for entity in &manifest.entities {
        if entity.name.starts_with('_') {
            continue; // framework-owned tables, admin-only at the route
        }
        let url = format!("{target}/api/entities/{}", entity.name);

        // Anonymous list. A read policy filters rows per caller, so a
        // 200 alone proves nothing; rows in it do.
        probes += 1;
        match probe_anon_get(&agent, &url) {
            ProbeResult::Status(200, body) => {
                let rows = list_row_count(&body);
                if rows > 0 && public_entities.contains(&entity.name) {
                    findings.push(Finding {
                        level: Level::Review,
                        target: url.clone(),
                        kind: "entity-public-read",
                        status: 200,
                        expected: "rows meant for anyone",
                        detail: format!(
                            "Entity \"{}\" is listed as public and returned {rows} row(s) to an anonymous GET. Confirm every field in them is meant for anyone.",
                            entity.name
                        ),
                    });
                } else if rows > 0 {
                    findings.push(Finding {
                        level: Level::Finding,
                        target: url.clone(),
                        kind: "entity-anonymous-read",
                        status: 200,
                        expected: "403 POLICY_DENIED, or 200 with no rows",
                        detail: format!(
                            "Entity \"{}\" returned {rows} row(s) to an anonymous GET. If the entity is public (PublicPost, ShareView, etc.), pass it in --public-entities. Otherwise its read policy is too permissive.",
                            entity.name
                        ),
                    });
                } else {
                    findings.push(Finding {
                        level: Level::Info,
                        target: url.clone(),
                        kind: "entity-anonymous-list-empty",
                        status: 200,
                        expected: "403 POLICY_DENIED, or 200 with no rows",
                        detail: format!(
                            "Entity \"{}\" answered an anonymous GET with zero rows. Its read policy filtered every row, or the table is empty. Seed a row on a dev server to tell the two apart.",
                            entity.name
                        ),
                    });
                }
            }
            ProbeResult::Status(..) => {}
            ProbeResult::ConnectionError(_) => {}
        }

        // Anonymous insert. Writes a row when the policy allows it, so
        // only on localhost or with --writes.
        if !writes {
            continue;
        }
        probes += 1;
        if let ProbeResult::Status(200 | 201, _) = probe_anon_post(&agent, &url, &origin, "{}") {
            findings.push(Finding {
                level: Level::Finding,
                target: url.clone(),
                kind: "entity-anonymous-insert",
                status: 200,
                expected: "403 POLICY_DENIED",
                detail: format!(
                    "Entity \"{}\" accepted an anonymous POST insert. Confirm the policy's `allowInsert` isn't `\"true\"` — `pylon lint` will catch this statically.",
                    entity.name
                ),
            });
        }
    }
    if !writes && manifest.entities.iter().any(|e| !e.name.starts_with('_')) {
        skipped.push(
            "entity inserts: the target is not localhost; pass --writes to probe them".into(),
        );
    }

    if json_mode {
        output::print_json(&JsonReport {
            target_base_url: &target,
            functions_source,
            probes,
            skipped: &skipped,
            findings: &findings,
        });
    } else {
        print_pretty(&target, functions_source, probes, &skipped, &findings);
    }

    if strict && findings.iter().any(|f| f.level == Level::Finding) {
        return ExitCode::Error;
    }
    ExitCode::Ok
}

#[derive(Debug, PartialEq, Eq)]
enum FnProbe {
    /// 401/403: the auth gate refused the anonymous call. 404: not
    /// exposed over HTTP (internal or not registered).
    Gated,
    /// Past the auth gate: the router parsed the body (400) or, on an
    /// older runtime, ran the function.
    Reachable,
    /// Rate limited, refused by the CSRF gate, or the runtime was not
    /// serving functions.
    Inconclusive,
}

fn classify_fn(status: u16, body: &str) -> FnProbe {
    match status {
        403 if body.contains("CSRF_REJECTED") => FnProbe::Inconclusive,
        401 | 403 | 404 => FnProbe::Gated,
        429 | 500..=599 => FnProbe::Inconclusive,
        _ => FnProbe::Reachable,
    }
}

fn fn_finding(f: &FnTarget, url: &str, status: u16) -> Finding {
    let what = match &f.kind {
        Some(kind) => format!("{kind} \"{}\"", f.name),
        None => format!("Function \"{}\"", f.name),
    };
    match f.auth.as_deref() {
        Some(auth @ ("public" | "guest")) => Finding {
            level: Level::Review,
            target: url.to_string(),
            kind: "fn-declared-public",
            status,
            expected: "an intentional `auth: \"public\"` or `\"guest\"`",
            detail: format!(
                "{what} is declared `auth: \"{auth}\"` and anyone can call it. Confirm it checks what it needs to (a signature, a token, the caller's ownership) before it reads or writes."
            ),
        },
        Some(auth) => Finding {
            level: Level::Finding,
            target: url.to_string(),
            kind: "fn-gate-bypassed",
            status,
            expected: "401 AUTH_REQUIRED (or 403 if admin-gated)",
            detail: format!(
                "{what} is declared `auth: \"{auth}\"` but answered an anonymous call with {status} instead of 401/403. The runtime's auth gate did not run. Upgrade @pylonsync/functions and the runtime, and report it."
            ),
        },
        None => Finding {
            level: Level::Review,
            target: url.to_string(),
            kind: "fn-anonymous-reachable",
            status,
            expected: "401 AUTH_REQUIRED (or 403 if admin-gated)",
            detail: format!(
                "{what} answered an anonymous call with {status}, past the auth gate. Either it is `auth: \"public\"` on purpose, or it should be `\"user\"`. Run with --admin-token to see its declared auth."
            ),
        },
    }
}

/// `GET /api/fn` with the admin token. None when the target does not
/// answer it (no token, wrong token, or an older runtime).
fn list_functions(
    agent: &ureq::Agent,
    target: &str,
    admin_token: Option<&str>,
) -> Option<Vec<FnTarget>> {
    let mut req = agent.get(&format!("{target}/api/fn"));
    if let Some(t) = admin_token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let body = req.call().ok()?.into_string().ok()?;
    parse_function_list(&body)
}

fn parse_function_list(body: &str) -> Option<Vec<FnTarget>> {
    let listed: Vec<ListedFn> = serde_json::from_str(body).ok()?;
    let mut out: Vec<FnTarget> = listed
        .into_iter()
        .filter(|f| !f.internal)
        .map(|f| FnTarget {
            name: f.name,
            kind: f.fn_type,
            // The runtime treats a missing auth as "user".
            auth: Some(f.auth.unwrap_or_else(|| "user".into())),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    Some(out)
}

/// The manifest's queries and actions, plus one function per file in
/// `functions/`. Declared auth is unknown here.
fn functions_from_manifest(manifest: &AppManifest, file_names: &[String]) -> Vec<FnTarget> {
    let mut names: Vec<String> = Vec::new();
    names.extend(manifest.queries.iter().map(|q| q.name.clone()));
    names.extend(manifest.actions.iter().map(|a| a.name.clone()));
    names.extend(file_names.iter().cloned());
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .map(|name| FnTarget {
            name,
            kind: None,
            auth: None,
        })
        .collect()
}

/// Function names from a `functions/` directory: the runtime registers
/// one function per top-level `.ts`/`.js` file, named after the file.
fn function_file_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = file
            .strip_suffix(".ts")
            .or_else(|| file.strip_suffix(".js"))
        else {
            continue;
        };
        // Tests and type declarations are not functions.
        if stem.ends_with(".test") || stem.ends_with(".spec") || stem.ends_with(".d") {
            continue;
        }
        out.push(stem.to_string());
    }
    out.sort_unstable();
    out
}

/// Rows in an entity list response (`{ "data": [...] }`).
fn list_row_count(body: &str) -> usize {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("data").and_then(|d| d.as_array()).map(|a| a.len()))
        .unwrap_or(0)
}

enum ProbeResult {
    Status(u16, String),
    ConnectionError(String),
}

/// Times a probe waits out a 429 before reporting it.
const RATE_LIMIT_RETRIES: u32 = 10;
/// Longest wait for one 429, whatever Retry-After says.
const MAX_RETRY_AFTER_SECS: u64 = 30;

/// Anonymous POST. No Authorization header. Returns the error body so a
/// CSRF refusal can be told apart from an auth refusal.
fn probe_anon_post(agent: &ureq::Agent, url: &str, origin: &str, body: &str) -> ProbeResult {
    with_rate_limit_retry(|| {
        agent
            .post(url)
            .set("Content-Type", "application/json")
            // Most pylon installs gate state-changing routes on the
            // CSRF plugin's `Origin` header check. Sending the target's
            // own origin keeps the test focused on AUTH gates. It must be
            // the origin (scheme://host[:port]), not the URL: the CSRF gate
            // refuses a URL with a path, and every probe then looked gated.
            .set("Origin", origin)
            .send_string(body)
            .map_err(Box::new)
    })
}

fn probe_anon_get(agent: &ureq::Agent, url: &str) -> ProbeResult {
    with_rate_limit_retry(|| agent.get(url).call().map_err(Box::new))
}

/// Send, and on a 429 wait for the server's Retry-After and send again.
/// A probe run is a burst of anonymous requests from one IP, which is
/// what a per-IP rate limit exists to slow down.
fn with_rate_limit_retry(
    send: impl Fn() -> Result<ureq::Response, Box<ureq::Error>>,
) -> ProbeResult {
    let mut attempt = 0;
    loop {
        match send().map_err(|e| *e) {
            Ok(resp) => {
                let status = resp.status();
                return ProbeResult::Status(status, resp.into_string().unwrap_or_default());
            }
            Err(ureq::Error::Status(429, resp)) if attempt < RATE_LIMIT_RETRIES => {
                let wait = resp
                    .header("Retry-After")
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .unwrap_or(1)
                    .clamp(1, MAX_RETRY_AFTER_SECS);
                std::thread::sleep(Duration::from_secs(wait));
                attempt += 1;
            }
            Err(ureq::Error::Status(code, resp)) => {
                return ProbeResult::Status(code, resp.into_string().unwrap_or_default());
            }
            Err(e) => return ProbeResult::ConnectionError(e.to_string()),
        }
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn pick_target(args: &[String]) -> String {
    flag_value(args, "--target")
        .unwrap_or_else(|| "http://localhost:4321".into())
        .trim_end_matches('/')
        .to_string()
}

/// `scheme://host[:port]` of the target, for the Origin header.
fn origin_of(target: &str) -> String {
    match target.split_once("://") {
        Some((scheme, rest)) => format!("{scheme}://{}", rest.split('/').next().unwrap_or(rest)),
        None => target.split('/').next().unwrap_or(target).to_string(),
    }
}

fn is_local(target: &str) -> bool {
    let rest = target.split("://").nth(1).unwrap_or(target);
    let host = rest.split(['/', ':']).next().unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

fn load_manifest() -> Result<AppManifest, String> {
    const CANDIDATES: &[&str] = &[
        "pylon.manifest.json",
        "apps/api/pylon.manifest.json",
        ".pylon/manifest.json",
    ];
    for path in CANDIDATES {
        if !Path::new(path).exists() {
            continue;
        }
        let raw =
            std::fs::read_to_string(path).map_err(|e| format!("Could not read {path}: {e}"))?;
        let manifest: AppManifest =
            serde_json::from_str(&raw).map_err(|e| format!("Could not parse {path}: {e}"))?;
        return Ok(manifest);
    }
    Err(format!(
        "No manifest found (checked: {}). Run `pylon dev` (or `pylon codegen app.ts --out pylon.manifest.json`) once, then re-run test:security.",
        CANDIDATES.join(", ")
    ))
}

fn print_pretty(
    target: &str,
    source: &str,
    probes: usize,
    skipped: &[String],
    findings: &[Finding],
) {
    println!();
    println!(
        "pylon test:security — target {target}, {probes} probe(s), functions from the {source}"
    );
    if source == "manifest" {
        println!("   Declared auth is unknown. Pass --admin-token (or set PYLON_ADMIN_TOKEN) to list functions from the server.");
    }
    println!();
    for s in skipped {
        println!("-  skipped: {s}");
    }
    if !skipped.is_empty() {
        println!();
    }
    if findings.is_empty() {
        println!("\u{2713}  No findings — every probed surface rejected anonymous access.");
        return;
    }
    for level in [Level::Finding, Level::Review, Level::Info] {
        for f in findings.iter().filter(|f| f.level == level) {
            let mark = match level {
                Level::Finding => "\u{26A0} ",
                Level::Review => "?  ",
                Level::Info => "i  ",
            };
            println!("{mark} {} [{}]", f.kind, f.target);
            println!("   status: {} (expected: {})", f.status, f.expected);
            println!("   {}", f.detail);
            println!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_statuses_and_reachable_statuses() {
        assert_eq!(classify_fn(401, ""), FnProbe::Gated);
        assert_eq!(
            classify_fn(403, r#"{"error":{"code":"FORBIDDEN"}}"#),
            FnProbe::Gated
        );
        assert_eq!(classify_fn(404, ""), FnProbe::Gated);
        // The router parses the body only after the auth gate passed.
        assert_eq!(classify_fn(400, ""), FnProbe::Reachable);
        assert_eq!(classify_fn(200, ""), FnProbe::Reachable);
        assert_eq!(classify_fn(429, ""), FnProbe::Inconclusive);
        assert_eq!(classify_fn(503, ""), FnProbe::Inconclusive);
        // A CSRF refusal says nothing about the auth gate.
        assert_eq!(
            classify_fn(403, r#"{"error":{"code":"CSRF_REJECTED"}}"#),
            FnProbe::Inconclusive
        );
    }

    #[test]
    fn server_list_drops_internal_and_defaults_auth_to_user() {
        let body = r#"[
            {"name":"b","fn_type":"mutation","auth":"public"},
            {"name":"a","fn_type":"query"},
            {"name":"helper","fn_type":"mutation","internal":true,"auth":"user"}
        ]"#;
        let list = parse_function_list(body).unwrap();
        assert_eq!(
            list,
            vec![
                FnTarget {
                    name: "a".into(),
                    kind: Some("query".into()),
                    auth: Some("user".into())
                },
                FnTarget {
                    name: "b".into(),
                    kind: Some("mutation".into()),
                    auth: Some("public".into())
                },
            ]
        );
        assert!(parse_function_list(r#"{"error":{"code":"FORBIDDEN"}}"#).is_none());
    }

    #[test]
    fn levels_follow_declared_auth() {
        let f = |auth: Option<&str>| FnTarget {
            name: "x".into(),
            kind: None,
            auth: auth.map(str::to_string),
        };
        assert_eq!(
            fn_finding(&f(Some("public")), "u", 400).level,
            Level::Review
        );
        assert_eq!(fn_finding(&f(Some("guest")), "u", 400).level, Level::Review);
        assert_eq!(fn_finding(&f(Some("user")), "u", 400).level, Level::Finding);
        assert_eq!(
            fn_finding(&f(Some("admin")), "u", 400).level,
            Level::Finding
        );
        assert_eq!(fn_finding(&f(None), "u", 400).level, Level::Review);
    }

    #[test]
    fn function_files_name_functions() {
        let dir = std::env::temp_dir().join(format!("pylon-test-security-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in [
            "createPost.ts",
            "listPosts.js",
            "createPost.test.ts",
            "types.d.ts",
            "README.md",
        ] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        assert_eq!(
            function_file_names(&dir),
            vec!["createPost".to_string(), "listPosts".to_string()]
        );
        assert!(function_file_names(&dir.join("missing")).is_empty());
    }

    #[test]
    fn list_rows_are_counted_from_data() {
        assert_eq!(
            list_row_count(r#"{"data":[{"id":"1"},{"id":"2"}],"total":2}"#),
            2
        );
        assert_eq!(list_row_count(r#"{"data":[]}"#), 0);
        assert_eq!(list_row_count("not json"), 0);
    }

    #[test]
    fn origin_is_scheme_and_host() {
        assert_eq!(
            origin_of("https://app.example.com"),
            "https://app.example.com"
        );
        assert_eq!(
            origin_of("http://localhost:4321/x/y"),
            "http://localhost:4321"
        );
    }

    #[test]
    fn local_targets() {
        assert!(is_local("http://localhost:4321"));
        assert!(is_local("http://127.0.0.1:4321/"));
        assert!(!is_local("https://app.example.com"));
        assert!(!is_local("https://localhost.example.com"));
    }
}
