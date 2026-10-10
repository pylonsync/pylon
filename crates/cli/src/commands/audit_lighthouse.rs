//! Lighthouse performance and accessibility runs for `pylon audit`.
//!
//! How Lighthouse runs: `bun x --bun lighthouse@<LIGHTHOUSE_VERSION>`, with
//! CHROME_PATH set to the Chrome or Chromium found here. The reasons:
//!
//! - Lighthouse exists only as an npm package. There is no Rust port, and a
//!   copy of its audits would drift from what Google measures.
//! - Every Pylon project already needs Bun, and the dev-env image has Bun but
//!   no Node, so `npx` is not an option there. `--bun` makes Bun run the
//!   package even though its bin asks for `node`. Lighthouse 13.5.0 runs
//!   correctly under Bun 1.3.
//! - The version is pinned, so two runs of one Pylon release score pages with
//!   the same audits. `bun x` keeps the package in Bun's cache after the
//!   first run; the first run needs the npm registry.
//! - The CLI flags and the JSON report (`categories`, `audits`) are
//!   Lighthouse's stable interface. Driving Chrome over CDP from Rust would
//!   mean writing Lighthouse again.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use pylon_kernel::{Diagnostic, Severity};

use super::audit_page::diag;

/// Bump on purpose, after a run against a real app with the new version.
pub const LIGHTHOUSE_VERSION: &str = "13.5.0";
/// A cold run on a slow machine takes about 30 seconds.
const RUN_TIMEOUT: Duration = Duration::from_secs(180);

/// Find a Chrome or Chromium binary.
///
/// Order: `CHROME_PATH` (Lighthouse's own variable), then
/// `AGENT_BROWSER_EXECUTABLE_PATH` (set in the dev-env image), then common
/// names on PATH, then the standard install locations on macOS and Windows.
pub fn find_chrome() -> Option<PathBuf> {
    for var in ["CHROME_PATH", "AGENT_BROWSER_EXECUTABLE_PATH"] {
        if let Some(p) = std::env::var_os(var).map(PathBuf::from) {
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let names = [
        "chromium",
        "chromium-browser",
        "google-chrome",
        "google-chrome-stable",
        "chrome",
    ];
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
                #[cfg(windows)]
                {
                    let exe = dir.join(format!("{name}.exe"));
                    if exe.is_file() {
                        return Some(exe);
                    }
                }
            }
        }
    }
    let fixed: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ]
    } else if cfg!(windows) {
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ]
    } else {
        &[]
    };
    fixed.iter().map(PathBuf::from).find(|p| p.is_file())
}

/// True when `bun` runs.
pub fn bun_available() -> bool {
    Command::new("bun")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Chrome flags for a headless run. Chrome's sandbox needs a setuid helper or
/// user namespaces, which containers often do not give; there it is turned
/// off, the same way agent-browser does it (root, a container, or
/// AGENT_BROWSER_ARGS with --no-sandbox).
fn chrome_flags() -> String {
    let mut flags = vec!["--headless=new"];
    if cfg!(target_os = "linux") {
        let in_container = Path::new("/.dockerenv").exists()
            || std::env::var("AGENT_BROWSER_ARGS").is_ok_and(|a| a.contains("--no-sandbox"));
        if in_container || is_root() {
            flags.push("--no-sandbox");
        }
        // /dev/shm is 64 MB in Docker by default; Chrome crashes on big pages.
        flags.push("--disable-dev-shm-usage");
    }
    flags.join(" ")
}

#[cfg(unix)]
fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
fn is_root() -> bool {
    false
}

/// Run Lighthouse against `url` and return its JSON report.
pub fn run_lighthouse(url: &str, chrome: &Path) -> Result<serde_json::Value, String> {
    let mut child = Command::new("bun")
        .args([
            "x",
            "--bun",
            &format!("lighthouse@{LIGHTHOUSE_VERSION}"),
            url,
            "--output=json",
            "--output-path=stdout",
            "--only-categories=performance,accessibility",
            "--quiet",
            &format!("--chrome-flags={}", chrome_flags()),
        ])
        .env("CHROME_PATH", chrome)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start bun: {e}"))?;

    // Read both pipes on threads so a full pipe never blocks the child.
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let out_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let deadline = Instant::now() + RUN_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(200)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "Lighthouse did not finish in {}s",
                    RUN_TIMEOUT.as_secs()
                ));
            }
            Err(e) => return Err(format!("could not wait for Lighthouse: {e}")),
        }
    };
    let out = out_thread.join().unwrap_or_default();
    let err = err_thread.join().unwrap_or_default();
    if !status.success() {
        let tail: Vec<&str> = err.lines().rev().take(5).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!(
            "Lighthouse exited with {status}: {}",
            tail.join(" | ")
        ));
    }
    serde_json::from_str(&out)
        .map_err(|e| format!("Lighthouse printed a report that does not parse: {e}"))
}

/// Scores and lab metrics from one report.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct LighthouseScores {
    /// 0 to 100.
    pub performance: Option<u32>,
    pub accessibility: Option<u32>,
    pub lcp_ms: Option<f64>,
    pub cls: Option<f64>,
    pub tbt_ms: Option<f64>,
}

/// Accessibility audits that the HTML checks already report, so a page does
/// not get two messages for one problem.
const COVERED_AUDITS: &[&str] = &[
    "image-alt",
    "html-has-lang",
    "document-title",
    "meta-viewport",
];

/// Core Web Vitals "poor" limits (web.dev/articles/vitals). TBT stands in for
/// INP in a lab run.
const LCP_POOR_MS: f64 = 4000.0;
const CLS_POOR: f64 = 0.25;
const TBT_POOR_MS: f64 = 600.0;

/// Turn a Lighthouse report into scores and diagnostics.
pub fn interpret(report: &serde_json::Value) -> (LighthouseScores, Vec<Diagnostic>) {
    let mut out = Vec::new();
    if let Some(err) = report
        .get("runtimeError")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
    {
        out.push(diag(
            Severity::Warning,
            "AUDIT_LH_RUNTIME_ERROR",
            format!("Lighthouse could not measure the page: {err}"),
            "Open the page in a browser and fix what stops it from rendering.",
        ));
        return (LighthouseScores::default(), out);
    }
    let score = |cat: &str| {
        report["categories"][cat]["score"]
            .as_f64()
            .map(|s| (s * 100.0).round() as u32)
    };
    let metric = |id: &str| report["audits"][id]["numericValue"].as_f64();
    let scores = LighthouseScores {
        performance: score("performance"),
        accessibility: score("accessibility"),
        lcp_ms: metric("largest-contentful-paint"),
        cls: metric("cumulative-layout-shift"),
        tbt_ms: metric("total-blocking-time"),
    };

    if let Some(perf) = scores.performance {
        if perf < 90 {
            let severity = if perf < 50 {
                Severity::Error
            } else {
                Severity::Warning
            };
            let fixes = top_performance_fixes(report);
            let fix = if fixes.is_empty() {
                "Run Lighthouse in Chrome DevTools on this page to see what is slow.".to_string()
            } else {
                format!("Fix first: {}.", fixes.join("; "))
            };
            out.push(Diagnostic {
                severity,
                code: "AUDIT_LH_PERFORMANCE".into(),
                message: format!("Performance score {perf} ({}).", metrics_summary(&scores)),
                span: None,
                hint: Some(fix),
            });
        }
    }
    if scores.lcp_ms.is_some_and(|v| v > LCP_POOR_MS) {
        out.push(diag(
            Severity::Error,
            "AUDIT_LH_LCP",
            format!("Largest Contentful Paint is {:.1} s. Google rates more than 4 s as poor.", scores.lcp_ms.unwrap_or_default() / 1000.0),
            "Make the largest image or text block load first: <Image priority>, smaller images, less blocking JS.",
        ));
    }
    if scores.cls.is_some_and(|v| v > CLS_POOR) {
        out.push(diag(
            Severity::Error,
            "AUDIT_LH_CLS",
            format!("Cumulative Layout Shift is {:.2}. Google rates more than 0.25 as poor.", scores.cls.unwrap_or_default()),
            "Give images and embeds a width and height, and do not insert content above what is on screen.",
        ));
    }
    if scores.tbt_ms.is_some_and(|v| v > TBT_POOR_MS) {
        out.push(diag(
            Severity::Error,
            "AUDIT_LH_TBT",
            format!(
                "Total Blocking Time is {:.0} ms. More than 600 ms is poor.",
                scores.tbt_ms.unwrap_or_default()
            ),
            "Ship less JavaScript on this page: move heavy components behind dynamic().",
        ));
    }

    if let Some(a11y) = scores.accessibility {
        if a11y < 70 {
            out.push(diag(
                Severity::Error,
                "AUDIT_LH_ACCESSIBILITY",
                format!("Accessibility score {a11y}."),
                "Fix the accessibility warnings below for this page.",
            ));
        }
    }
    out.extend(failed_accessibility_audits(report));
    (scores, out)
}

fn metrics_summary(s: &LighthouseScores) -> String {
    let mut parts = Vec::new();
    if let Some(v) = s.lcp_ms {
        parts.push(format!("LCP {:.1} s", v / 1000.0));
    }
    if let Some(v) = s.cls {
        parts.push(format!("CLS {v:.2}"));
    }
    if let Some(v) = s.tbt_ms {
        parts.push(format!("TBT {v:.0} ms"));
    }
    parts.join(", ")
}

/// Titles of the performance audits that failed, largest estimated saving
/// first, at most three.
fn top_performance_fixes(report: &serde_json::Value) -> Vec<String> {
    let Some(refs) = report["categories"]["performance"]["auditRefs"].as_array() else {
        return Vec::new();
    };
    let mut failed: Vec<(f64, String)> = refs
        .iter()
        // "metrics" are the measurements themselves and "hidden" audits are
        // inputs to other audits; neither is a fix.
        .filter(|r| matches!(r["group"].as_str(), Some("insights") | Some("diagnostics")))
        .filter_map(|r| {
            let audit = &report["audits"][r["id"].as_str()?];
            if !matches!(
                audit["scoreDisplayMode"].as_str(),
                Some("numeric") | Some("binary") | Some("metricSavings")
            ) {
                return None;
            }
            let score = audit["score"].as_f64()?;
            if score >= 0.9 {
                return None;
            }
            let savings = audit["metricSavings"]
                .as_object()
                .map(|m| m.values().filter_map(|v| v.as_f64()).sum::<f64>())
                .unwrap_or(0.0);
            Some((savings, audit["title"].as_str()?.to_string()))
        })
        .collect();
    failed.sort_by(|a, b| b.0.total_cmp(&a.0));
    failed.into_iter().take(3).map(|(_, t)| t).collect()
}

/// One warning per accessibility audit that failed, with the audit's own
/// advice and its reference link as the fix.
fn failed_accessibility_audits(report: &serde_json::Value) -> Vec<Diagnostic> {
    let Some(refs) = report["categories"]["accessibility"]["auditRefs"].as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for r in refs {
        let Some(id) = r["id"].as_str() else { continue };
        if r["weight"].as_f64().unwrap_or(0.0) == 0.0 || COVERED_AUDITS.contains(&id) {
            continue;
        }
        let audit = &report["audits"][id];
        if audit["scoreDisplayMode"].as_str() != Some("binary")
            || audit["score"].as_f64() != Some(0.0)
        {
            continue;
        }
        let title = audit["title"].as_str().unwrap_or(id);
        let items = audit["details"]["items"].as_array().map_or(0, Vec::len);
        let message = if items > 0 {
            format!("{title} ({items} element(s)).")
        } else {
            format!("{title}.")
        };
        out.push(Diagnostic {
            severity: Severity::Warning,
            code: format!(
                "AUDIT_LH_A11Y_{}",
                id.to_ascii_uppercase().replace('-', "_")
            ),
            message,
            span: None,
            hint: Some(describe_fix(
                audit["description"].as_str().unwrap_or_default(),
            )),
        });
    }
    out
}

/// Lighthouse descriptions are Markdown: "Text. [Learn how...](url)." Keep
/// the first sentence and the link URL.
fn describe_fix(description: &str) -> String {
    let link = description.find("](").and_then(|start| {
        let rest = &description[start + 2..];
        rest.find(')').map(|end| rest[..end].to_string())
    });
    let text = match description.find('[') {
        Some(i) => &description[..i],
        None => description,
    };
    let first = text
        .split_inclusive(". ")
        .next()
        .unwrap_or(text)
        .trim()
        .to_string();
    match link {
        Some(url) if first.is_empty() => format!("See {url}"),
        Some(url) => format!("{first} See {url}"),
        None => first,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(perf: f64, a11y: f64) -> serde_json::Value {
        serde_json::json!({
            "lighthouseVersion": "13.5.0",
            "categories": {
                "performance": {
                    "score": perf,
                    "auditRefs": [
                        { "id": "largest-contentful-paint", "weight": 25, "group": "metrics" },
                        { "id": "render-blocking-insight", "weight": 0, "group": "insights" },
                        { "id": "image-delivery-insight", "weight": 0, "group": "insights" },
                        { "id": "font-display-insight", "weight": 0, "group": "insights" },
                        { "id": "screenshot-thumbnails", "weight": 0, "group": "hidden" }
                    ]
                },
                "accessibility": {
                    "score": a11y,
                    "auditRefs": [
                        { "id": "color-contrast", "weight": 7 },
                        { "id": "image-alt", "weight": 10 },
                        { "id": "button-name", "weight": 10 },
                        { "id": "landmark-one-main", "weight": 0 }
                    ]
                }
            },
            "audits": {
                "largest-contentful-paint": { "score": 0.2, "numericValue": 5200.0, "title": "Largest Contentful Paint" },
                "cumulative-layout-shift": { "score": 1, "numericValue": 0.01 },
                "total-blocking-time": { "score": 1, "numericValue": 120.0 },
                "render-blocking-insight": { "score": 0, "scoreDisplayMode": "metricSavings", "title": "Render blocking requests", "metricSavings": { "FCP": 300, "LCP": 300 } },
                "image-delivery-insight": { "score": 0.5, "scoreDisplayMode": "metricSavings", "title": "Improve image delivery", "metricSavings": { "LCP": 1200 } },
                "font-display-insight": { "score": 1, "scoreDisplayMode": "metricSavings", "title": "Font display" },
                "screenshot-thumbnails": { "score": 0, "scoreDisplayMode": "informative", "title": "Thumbnails" },
                "color-contrast": {
                    "score": 0, "scoreDisplayMode": "binary",
                    "title": "Background and foreground colors do not have a sufficient contrast ratio.",
                    "description": "Low-contrast text is difficult or impossible for many users to read. [Learn how to provide sufficient color contrast](https://dequeuniversity.com/rules/axe/4.10/color-contrast).",
                    "details": { "items": [{}, {}] }
                },
                "image-alt": { "score": 0, "scoreDisplayMode": "binary", "title": "Image elements do not have [alt] attributes" },
                "button-name": { "score": 1, "scoreDisplayMode": "binary", "title": "Buttons have an accessible name" },
                "landmark-one-main": { "score": 0, "scoreDisplayMode": "binary", "title": "Document does not have a main landmark." }
            }
        })
    }

    #[test]
    fn a_slow_page_is_an_error_with_the_biggest_fixes_first() {
        let (scores, d) = interpret(&report(0.42, 0.95));
        assert_eq!(scores.performance, Some(42));
        assert_eq!(scores.accessibility, Some(95));
        let perf = d.iter().find(|d| d.code == "AUDIT_LH_PERFORMANCE").unwrap();
        assert_eq!(perf.severity, Severity::Error);
        assert!(perf.message.contains("LCP 5.2 s"), "{}", perf.message);
        assert_eq!(
            perf.hint.as_deref(),
            Some("Fix first: Improve image delivery; Render blocking requests.")
        );
        let lcp = d.iter().find(|d| d.code == "AUDIT_LH_LCP").unwrap();
        assert_eq!(lcp.severity, Severity::Error);
    }

    #[test]
    fn a_middling_page_is_a_warning_and_a_fast_one_is_silent() {
        let mut r = report(0.75, 1.0);
        r["audits"]["largest-contentful-paint"]["numericValue"] = 2000.0.into();
        let (_, d) = interpret(&r);
        let perf = d.iter().find(|d| d.code == "AUDIT_LH_PERFORMANCE").unwrap();
        assert_eq!(perf.severity, Severity::Warning);

        let mut r = report(0.95, 1.0);
        r["audits"]["largest-contentful-paint"]["numericValue"] = 1500.0.into();
        r["audits"]["color-contrast"]["score"] = 1.into();
        let (_, d) = interpret(&r);
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn failed_accessibility_audits_become_warnings() {
        let (_, d) = interpret(&report(0.95, 0.6));
        let codes: Vec<&str> = d.iter().map(|d| d.code.as_str()).collect();
        // image-alt is covered by the HTML check; landmark-one-main has no
        // weight; button-name passed.
        assert!(codes.contains(&"AUDIT_LH_A11Y_COLOR_CONTRAST"), "{codes:?}");
        assert!(!codes.contains(&"AUDIT_LH_A11Y_IMAGE_ALT"));
        assert!(!codes.contains(&"AUDIT_LH_A11Y_LANDMARK_ONE_MAIN"));
        assert!(!codes.contains(&"AUDIT_LH_A11Y_BUTTON_NAME"));
        let a11y = d
            .iter()
            .find(|d| d.code == "AUDIT_LH_ACCESSIBILITY")
            .unwrap();
        assert_eq!(a11y.severity, Severity::Error);
        let contrast = d
            .iter()
            .find(|d| d.code == "AUDIT_LH_A11Y_COLOR_CONTRAST")
            .unwrap();
        assert!(
            contrast.message.ends_with("(2 element(s))."),
            "{}",
            contrast.message
        );
        assert_eq!(
            contrast.hint.as_deref(),
            Some("Low-contrast text is difficult or impossible for many users to read. See https://dequeuniversity.com/rules/axe/4.10/color-contrast")
        );
    }

    #[test]
    fn a_runtime_error_is_reported_once() {
        let r = serde_json::json!({ "runtimeError": { "code": "NO_FCP", "message": "The page did not paint any content." } });
        let (scores, d) = interpret(&r);
        assert!(scores.performance.is_none());
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "AUDIT_LH_RUNTIME_ERROR");
    }
}
