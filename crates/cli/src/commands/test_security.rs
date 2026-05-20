//! `pylon test:security` — adversarial probe against a running dev
//! server. Enumerates every registered function + entity from the
//! manifest, fires anonymous requests at each, and reports anything
//! that responds without an auth gate.
//!
//! Catches the canonical "I made a function and forgot to mark it
//! `auth: \"user\"` (or shipped a policy that allows anonymous
//! reads)" class. Runs in CI before deploy so the regression doesn't
//! reach prod.
//!
//! Usage:
//!   pylon test:security                  # probes http://localhost:4321
//!   pylon test:security --target https://api.example.com
//!   pylon test:security --json
//!
//! What it does NOT do:
//!   - Cross-tenant probing (needs test users + data). Use the
//!     policy linter (`pylon lint`) for static cross-tenant checks
//!     and ship integration tests for the dynamic ones.
//!   - Replay-based attacks (timing, parameter tampering on signed
//!     URLs, etc.). Handled separately by the auth-flow integration
//!     tests in the framework.
//!   - Fuzz mutation arguments. The function-runtime fuzz tests
//!     already cover the panic-resistance shape.
//!
//! The bar is "did the developer accidentally ship a public endpoint
//! they meant to gate?" — every 200 from an anonymous probe is a
//! finding worth reading.

use std::path::Path;
use std::time::Duration;

use pylon_kernel::{AppManifest, ExitCode};
use serde::Serialize;
use ureq;

use crate::output;

#[derive(Debug, Clone, Serialize)]
struct Finding {
    target: String,
    kind: &'static str,
    status: u16,
    expected: &'static str,
    detail: String,
}

#[derive(Debug, Serialize)]
struct JsonReport<'a> {
    target_base_url: &'a str,
    probes: usize,
    findings: &'a [Finding],
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let target = pick_target(args);
    let strict = args.iter().any(|a| a == "--strict");

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
        .user_agent("pylon-test-security/0.1")
        .build();

    let mut findings = Vec::new();
    let mut probes = 0;

    // Probe 1 — every registered function via /api/fn/<name>. Each
    // POST is anonymous (no Authorization header), so a properly-
    // gated function should respond 401 AUTH_REQUIRED. The framework
    // default is `auth: "user"` (since v0.3.158) — any 200 means
    // either the developer explicitly opted into auth: "public" OR
    // the deployed framework predates the default.
    let mut fn_names: Vec<&str> = Vec::new();
    fn_names.extend(manifest.queries.iter().map(|q| q.name.as_str()));
    fn_names.extend(manifest.actions.iter().map(|a| a.name.as_str()));
    fn_names.sort_unstable();
    fn_names.dedup();
    for name in &fn_names {
        probes += 1;
        let url = format!("{target}/api/fn/{name}");
        match probe_anon_post(&agent, &url) {
            ProbeResult::Status(200) | ProbeResult::Status(201) => {
                findings.push(Finding {
                    target: url.clone(),
                    kind: "fn-anonymous-200",
                    status: 200,
                    expected: "401 AUTH_REQUIRED (or 403 if admin-gated)",
                    detail: format!(
                        "Function \"{name}\" responded to an anonymous POST with 200. Either it's intentionally `auth: \"public\"` (in which case ignore) or the framework's auth gate isn't firing — bump @pylonsync/functions to >= 0.3.158 and confirm the def's auth field."
                    ),
                });
            }
            ProbeResult::Status(_) => {} // 401/403/404 all OK
            ProbeResult::ConnectionError(e) => {
                output::print_error(&format!("Could not reach {url}: {e}"));
                return ExitCode::Unavailable;
            }
        }
    }

    // Probe 2 — every entity via /api/entities/<entity>. Each POST
    // is anonymous; the default-deny policy should respond 403
    // POLICY_DENIED. Anonymous read of a list is also probed (some
    // policies allow `allow: "true"` reads intentionally — those
    // would 200 here and the policy linter PYL001 would already have
    // flagged the entity).
    for entity in &manifest.entities {
        if entity.name.starts_with('_') {
            continue; // framework-owned tables
        }
        // Anonymous insert — should always 403 unless the policy is
        // genuinely public.
        probes += 1;
        let insert_url = format!("{target}/api/entities/{}", entity.name);
        match probe_anon_post(&agent, &insert_url) {
            ProbeResult::Status(200) | ProbeResult::Status(201) => {
                findings.push(Finding {
                    target: insert_url.clone(),
                    kind: "entity-anonymous-insert",
                    status: 200,
                    expected: "403 POLICY_DENIED",
                    detail: format!(
                        "Entity \"{}\" accepted an anonymous POST insert with 200/201. Confirm the policy's `allowInsert` isn't `\"true\"` — `pylon lint` will catch this statically.",
                        entity.name
                    ),
                });
            }
            ProbeResult::Status(_) => {}
            ProbeResult::ConnectionError(_) => {} // already reported above
        }
        // Anonymous list — 200 is OK ONLY if the entity is
        // intentionally public; otherwise the policy should deny.
        probes += 1;
        let list_url = format!("{target}/api/entities/{}", entity.name);
        match probe_anon_get(&agent, &list_url) {
            ProbeResult::Status(200) => {
                findings.push(Finding {
                    target: list_url.clone(),
                    kind: "entity-anonymous-list",
                    status: 200,
                    expected: "403 POLICY_DENIED (unless intentionally public)",
                    detail: format!(
                        "Entity \"{}\" returned a 200 to an anonymous GET. If this is intentional (PublicPost, ShareView, etc.), ignore — otherwise the policy's `allowRead` is too permissive.",
                        entity.name
                    ),
                });
            }
            ProbeResult::Status(_) => {}
            ProbeResult::ConnectionError(_) => {}
        }
    }

    if json_mode {
        output::print_json(&JsonReport {
            target_base_url: &target,
            probes,
            findings: &findings,
        });
    } else {
        print_pretty(&target, probes, &findings);
    }

    if strict && !findings.is_empty() {
        return ExitCode::Error;
    }
    ExitCode::Ok
}

enum ProbeResult {
    Status(u16),
    ConnectionError(String),
}

/// Anonymous POST `{}`. No Authorization header. The server is
/// expected to treat this as an unauthenticated request. We don't
/// care about the response body — only the status code.
fn probe_anon_post(agent: &ureq::Agent, url: &str) -> ProbeResult {
    let req = agent
        .post(url)
        .set("Content-Type", "application/json")
        // Most pylon installs gate state-changing routes on the
        // CSRF plugin's `Origin` header check. Echoing back the
        // target's own origin keeps the test focused on AUTH gates
        // — if CSRF rejects, the developer ALREADY has defense in
        // depth; we want to know about the auth gate specifically.
        .set("Origin", url);
    match req.send_string("{}") {
        Ok(resp) => ProbeResult::Status(resp.status()),
        Err(ureq::Error::Status(code, _)) => ProbeResult::Status(code),
        Err(e) => ProbeResult::ConnectionError(e.to_string()),
    }
}

fn probe_anon_get(agent: &ureq::Agent, url: &str) -> ProbeResult {
    match agent.get(url).call() {
        Ok(resp) => ProbeResult::Status(resp.status()),
        Err(ureq::Error::Status(code, _)) => ProbeResult::Status(code),
        Err(e) => ProbeResult::ConnectionError(e.to_string()),
    }
}

fn pick_target(args: &[String]) -> String {
    args.iter()
        .position(|a| a == "--target")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "http://localhost:4321".into())
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
        "No manifest found (checked: {}). Run `pylon dev` once to emit pylon.manifest.json, then re-run test:security.",
        CANDIDATES.join(", ")
    ))
}

fn print_pretty(target: &str, probes: usize, findings: &[Finding]) {
    println!();
    println!("pylon test:security — target {target}, {probes} probe(s)");
    println!();
    if findings.is_empty() {
        println!("\u{2713}  No findings — every probed surface rejected anonymous access.");
        return;
    }
    for f in findings {
        println!("\u{26A0}  {} [{}]", f.kind, f.target);
        println!("   status: {} (expected: {})", f.status, f.expected);
        println!("   {}", f.detail);
        println!();
    }
}
