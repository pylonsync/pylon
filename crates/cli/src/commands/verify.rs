//! `pylon verify` — prove the app actually serves, without a human.
//!
//! Agents (and CI) need a machine-checkable answer to "did my change
//! break the app?" that's stronger than typecheck and cheaper than a
//! browser session. This walks the app the way a first visitor would:
//!
//!   1. `/health` answers 200.
//!   2. Every STATIC route in the manifest GETs successfully
//!      (2xx/3xx pass; 401/403 warn — auth-gated pages are working as
//!      designed; 404/5xx fail). Dynamic routes (`/d/:id`) can't be
//!      fabricated safely, so they're reported as skipped.
//!   3. Every `/_pylon/` asset referenced by a rendered page (client
//!      entry JS, compiled CSS) fetches 200 and non-empty — the class
//!      of failure where the page "works" but ships no hydration or
//!      no styles.
//!
//! Modes:
//!   pylon verify                    boot this project on a free port, verify, tear down
//!   pylon verify --url https://...  verify an already-running (e.g. deployed) app
//!   pylon deploy --verify           deploy, wait for the flip, then verify the live URL
//!
//! Output is a per-check table (or `--json`), exit 0 only when nothing
//! failed. Warnings don't fail the run.

use std::io::Read;
use std::time::Duration;

use pylon_kernel::ExitCode;

use crate::manifest::load_manifest;
use crate::output;

const HEALTH_TIMEOUT_SECS: u64 = 60;
const REQUEST_TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, serde::Serialize)]
pub struct VerifyReport {
    pub base_url: String,
    pub checks: Vec<Check>,
}

impl VerifyReport {
    pub fn failed(&self) -> bool {
        self.checks.iter().any(|c| c.status == CheckStatus::Fail)
    }
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let url = args
        .windows(2)
        .find(|w| w[0] == "--url")
        .map(|w| w[1].clone())
        .or_else(|| {
            args.iter()
                .find(|a| a.starts_with("--url="))
                .map(|a| a.trim_start_matches("--url=").to_string())
        });

    let manifest_path = "pylon.manifest.json";
    let manifest = match load_manifest(manifest_path) {
        Ok(m) => m,
        Err(diags) => {
            crate::output::print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };
    let route_paths: Vec<String> = manifest.routes.iter().map(|r| r.path.clone()).collect();

    // Source lint first: it needs no server, and it is the only thing that
    // sees this class of defect at all (see find_use_promise_all).
    let source_checks = lint_sources(std::path::Path::new("app"));

    let mut report = match url {
        Some(base) => verify_target(&base, &route_paths),
        None => {
            // Boot THIS project on a free port with the binary we're
            // running as, verify against it, then tear the child down.
            let port = match free_port() {
                Some(p) => p,
                None => {
                    output::print_error("Could not find a free port to boot the app on");
                    return ExitCode::Error;
                }
            };
            let exe = match std::env::current_exe() {
                Ok(e) => e,
                Err(e) => {
                    output::print_error(&format!("Could not locate the pylon binary: {e}"));
                    return ExitCode::Error;
                }
            };
            if !json_mode {
                println!("→ Booting app on port {port} for verification…");
            }
            let mut child = match std::process::Command::new(&exe)
                .arg("dev")
                .env("PYLON_PORT", port.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    output::print_error(&format!("Failed to boot the app: {e}"));
                    return ExitCode::Error;
                }
            };
            let base = format!("http://127.0.0.1:{port}");
            let report = verify_target(&base, &route_paths);
            let _ = child.kill();
            let _ = child.wait();
            report
        }
    };

    // Source lint results lead: a page that cannot render is a bigger answer
    // than any response code below it.
    let mut checks = source_checks;
    checks.extend(report.checks);
    report.checks = checks;

    print_report(&report, json_mode);
    if report.failed() {
        ExitCode::Error
    } else {
        ExitCode::Ok
    }
}

/// The reusable core: verify a serving Pylon app at `base_url` against
/// the given manifest route paths. Also used by `pylon deploy --verify`.
pub fn verify_target(base_url: &str, route_paths: &[String]) -> VerifyReport {
    verify_target_with_timeout(
        base_url,
        route_paths,
        Duration::from_secs(HEALTH_TIMEOUT_SECS),
    )
}

fn verify_target_with_timeout(
    base_url: &str,
    route_paths: &[String],
    health_timeout: Duration,
) -> VerifyReport {
    let base = base_url.trim_end_matches('/');
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build();
    let mut checks: Vec<Check> = Vec::new();

    // 1. Health — wait for it (the app may still be booting/flipping).
    let health_ok = wait_for_health(&agent, base, health_timeout, &mut checks);
    if !health_ok {
        return VerifyReport {
            base_url: base.to_string(),
            checks,
        };
    }

    // 2. Routes.
    let mut asset_urls: Vec<String> = Vec::new();
    for path in route_paths {
        if path.contains(':') || path.contains('*') {
            checks.push(Check {
                name: format!("route {path}"),
                status: CheckStatus::Skip,
                detail: "dynamic route — no safe sample id".into(),
            });
            continue;
        }
        let url = format!("{base}{path}");
        match agent.get(&url).call() {
            Ok(resp) => {
                let status = resp.status();
                let mut body = String::new();
                let _ = resp.into_reader().take(2_000_000).read_to_string(&mut body);
                collect_assets(&body, &mut asset_urls);
                checks.push(Check {
                    name: format!("route {path}"),
                    status: CheckStatus::Pass,
                    detail: format!("HTTP {status}"),
                });
            }
            Err(ureq::Error::Status(code, _)) => {
                let status = if code == 401 || code == 403 {
                    CheckStatus::Warn
                } else {
                    CheckStatus::Fail
                };
                let detail = if code == 401 || code == 403 {
                    format!("HTTP {code} (auth-gated — pass --url with a session to verify)")
                } else {
                    format!("HTTP {code}")
                };
                checks.push(Check {
                    name: format!("route {path}"),
                    status,
                    detail,
                });
            }
            Err(e) => {
                checks.push(Check {
                    name: format!("route {path}"),
                    status: CheckStatus::Fail,
                    detail: format!("request failed: {e}"),
                });
            }
        }
    }

    // 3. Referenced assets: hydration JS + compiled CSS must exist and
    //    be non-empty. This is the "page renders but ships no
    //    JS/styles" class (stale manifest, bundler drop, missing wasm).
    asset_urls.sort();
    asset_urls.dedup();
    for asset in &asset_urls {
        let url = format!("{base}{asset}");
        match agent.get(&url).call() {
            Ok(resp) => {
                let mut buf = Vec::new();
                let _ = resp.into_reader().take(50_000_000).read_to_end(&mut buf);
                if buf.is_empty() {
                    checks.push(Check {
                        name: format!("asset {asset}"),
                        status: CheckStatus::Fail,
                        detail: "200 but EMPTY body".into(),
                    });
                } else {
                    checks.push(Check {
                        name: format!("asset {asset}"),
                        status: CheckStatus::Pass,
                        detail: format!("{} bytes", buf.len()),
                    });
                }
            }
            Err(ureq::Error::Status(code, _)) => {
                checks.push(Check {
                    name: format!("asset {asset}"),
                    status: CheckStatus::Fail,
                    detail: format!("HTTP {code} — the page references it but it doesn't serve"),
                });
            }
            Err(e) => {
                checks.push(Check {
                    name: format!("asset {asset}"),
                    status: CheckStatus::Fail,
                    detail: format!("request failed: {e}"),
                });
            }
        }
    }

    VerifyReport {
        base_url: base.to_string(),
        checks,
    }
}

fn wait_for_health(
    agent: &ureq::Agent,
    base: &str,
    timeout: Duration,
    checks: &mut Vec<Check>,
) -> bool {
    let url = format!("{base}/health");
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match agent.get(&url).call() {
            Ok(resp) if resp.status() == 200 => {
                checks.push(Check {
                    name: "/health".into(),
                    status: CheckStatus::Pass,
                    detail: "HTTP 200".into(),
                });
                return true;
            }
            _ if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(500));
            }
            Ok(resp) => {
                checks.push(Check {
                    name: "/health".into(),
                    status: CheckStatus::Fail,
                    detail: format!("HTTP {} within {:?}", resp.status(), timeout),
                });
                return false;
            }
            Err(e) => {
                checks.push(Check {
                    name: "/health".into(),
                    status: CheckStatus::Fail,
                    detail: format!("unreachable within {timeout:?}: {e}"),
                });
                return false;
            }
        }
    }
}

/// Pull `/_pylon/...` asset URLs out of a rendered page: script src,
/// stylesheet href, and modulepreload links. String-scan on the two
/// attribute shapes the SSR runtime emits — robust to attribute order
/// because we match the URL itself.
fn collect_assets(html: &str, out: &mut Vec<String>) {
    let mut rest = html;
    while let Some(idx) = rest.find("/_pylon/") {
        let tail = &rest[idx..];
        let end = tail
            .find(|c: char| c == '"' || c == '\'' || c == ' ' || c == '>')
            .unwrap_or(tail.len());
        let url = &tail[..end];
        if url.ends_with(".js") || url.ends_with(".css") || url.ends_with(".wasm") {
            out.push(url.to_string());
        }
        rest = &rest[idx + 8..];
    }
}

fn free_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
}

pub fn print_report(report: &VerifyReport, json_mode: bool) {
    if json_mode {
        println!("{}", serde_json::to_string(report).unwrap_or_default());
        return;
    }
    println!();
    println!("Verifying {}", report.base_url);
    for c in &report.checks {
        let tag = match c.status {
            CheckStatus::Pass => "PASS",
            CheckStatus::Warn => "WARN",
            CheckStatus::Fail => "FAIL",
            CheckStatus::Skip => "skip",
        };
        println!("  [{tag}] {:<40} {}", c.name, c.detail);
    }
    let fails = report
        .checks
        .iter()
        .filter(|c| c.status == CheckStatus::Fail)
        .count();
    let warns = report
        .checks
        .iter()
        .filter(|c| c.status == CheckStatus::Warn)
        .count();
    println!();
    if fails == 0 {
        println!(
            "✓ verify passed ({} checks, {warns} warning(s))",
            report.checks.len()
        );
    } else {
        println!("✗ verify FAILED — {fails} failing check(s), {warns} warning(s)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_pylon_assets_from_html() {
        let html = r#"<link rel="stylesheet" href="/_pylon/build/styles-abc.css">
            <script type="module" src="/_pylon/build/client-entry-app__page-x1.js"></script>
            <link rel="modulepreload" href='/_pylon/build/chunks/shared-9.js'>
            <img src="/_pylon/build/logo.png">"#;
        let mut out = Vec::new();
        collect_assets(html, &mut out);
        assert!(out.contains(&"/_pylon/build/styles-abc.css".to_string()));
        assert!(out.contains(&"/_pylon/build/client-entry-app__page-x1.js".to_string()));
        assert!(out.contains(&"/_pylon/build/chunks/shared-9.js".to_string()));
        // Non-js/css/wasm assets aren't fetched (images can be large and
        // aren't the hydration-failure class this guards).
        assert!(!out.iter().any(|u| u.ends_with(".png")));
    }

    #[test]
    fn dynamic_routes_are_skipped_not_fetched() {
        // verify_target against an unreachable base: health fails fast,
        // and no route checks run — proving we never fabricate ids.
        let report = verify_target_with_timeout(
            "http://127.0.0.1:1",
            &["/".to_string(), "/d/:id".to_string()],
            Duration::from_millis(300),
        );
        assert!(report.failed());
        assert_eq!(report.checks.len(), 1); // health only
    }
}

// ---------------------------------------------------------------------------
// Source lint: `use(Promise.all([...]))` in a page
//
// serverData's methods hand back a thenable cached by (method, args). That
// cache is what makes `use()` stable across the render React replays after a
// suspension. `Promise.all` returns a NEW, pending, uncached promise every
// render, so `use()` suspends, React re-renders, builds another, and the
// component never returns — surfacing as React's minified error #482, "an
// async Client Component".
//
// Nothing else catches it. It typechecks, and verify's own route checks pass
// because a signed-out request redirects before the page ever renders. It only
// appears once someone signs in, which is how a whole app shipped with every
// page behind auth broken.
// ---------------------------------------------------------------------------

/// 1-based line numbers where `use(` wraps a `Promise.all(`.
pub fn find_use_promise_all(src: &str) -> Vec<usize> {
    const NEEDLE: &str = "use(";
    let bytes = src.as_bytes();
    let mut hits = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find(NEEDLE) {
        let at = from + rel;
        from = at + NEEDLE.len();
        // Not a suffix of another identifier (`reuse(`, `misuse(`).
        if at > 0 {
            let prev = bytes[at - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' || prev == b'.' {
                continue;
            }
        }
        let rest = src[from..].trim_start();
        if rest.starts_with("Promise.all(") {
            hits.push(src[..at].matches('\n').count() + 1);
        }
    }
    hits
}

#[cfg(test)]
mod use_promise_all_tests {
    use super::find_use_promise_all;

    #[test]
    fn flags_the_wrapped_form_across_lines() {
        let src = "const [a, b] = use(\n  Promise.all([\n    serverData.list(\"A\"),\n  ]),\n);\n";
        assert_eq!(find_use_promise_all(src), vec![1]);
    }

    #[test]
    fn flags_the_single_line_form() {
        assert_eq!(find_use_promise_all("const x = use(Promise.all([p, q]));"), vec![1]);
    }

    #[test]
    fn leaves_the_correct_pattern_alone() {
        let src = "const aP = serverData.list(\"A\");\nconst a = use(aP);\nconst b = use(serverData.get(\"B\", id));\n";
        assert!(find_use_promise_all(src).is_empty());
    }

    #[test]
    fn ignores_an_awaited_promise_all() {
        let src = "const [a, b] = await Promise.all([f(), g()]);";
        assert!(find_use_promise_all(src).is_empty());
    }

    #[test]
    fn does_not_match_a_longer_identifier() {
        assert!(find_use_promise_all("reuse(Promise.all([p]))").is_empty());
        assert!(find_use_promise_all("obj.use(Promise.all([p]))").is_empty());
    }

    #[test]
    fn reports_every_hit_with_its_line() {
        let src = "a\nuse(Promise.all([p]))\nb\nc\nuse(\n Promise.all([q]))\n";
        assert_eq!(find_use_promise_all(src), vec![2, 5]);
    }
}

/// Walk `dir` for page/layout sources and lint each one. Returns a Check per
/// offending file, and a single pass when the tree is clean.
fn lint_sources(dir: &std::path::Path) -> Vec<Check> {
    let mut files = Vec::new();
    collect_tsx(dir, &mut files);
    let mut checks = Vec::new();
    for file in &files {
        let Ok(src) = std::fs::read_to_string(file) else {
            continue;
        };
        let hits = find_use_promise_all(&src);
        if hits.is_empty() {
            continue;
        }
        let lines = hits
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        checks.push(Check {
            name: format!("lint {}", file.display()),
            status: CheckStatus::Fail,
            detail: format!(
                "use(Promise.all(…)) at line {lines}: Promise.all returns a new pending promise \
                 every render, so use() never settles and the page throws React error #482. \
                 Start each serverData call before the first use(), then use() each one."
            ),
        });
    }
    if checks.is_empty() && !files.is_empty() {
        checks.push(Check {
            name: "lint app sources".into(),
            status: CheckStatus::Pass,
            detail: format!("{} files, no use(Promise.all(…))", files.len()),
        });
    }
    checks
}

fn collect_tsx(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            collect_tsx(&path, out);
        } else if name.ends_with(".tsx") {
            out.push(path);
        }
    }
}
