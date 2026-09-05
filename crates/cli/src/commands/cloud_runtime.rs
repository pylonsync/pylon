//! `pylon runtime` — which pylon binary a Cloud project runs on.
//!
//! Normally there is nothing to manage here. A deploy boots the machine on the
//! runtime matching the `@pylonsync/*` version in the app's package.json, so
//! the binary and the packages that speak the function protocol to it are
//! always the same release. Change the dependency, deploy, done.
//!
//! The explicit form stays for the cases that need it: moving a machine
//! without a rebuild (a binary-only security fix), or holding one on a
//! specific version while investigating.
//!
//!   pylon runtime                 # show the version this project is running
//!   pylon runtime 0.4.7           # move it now, without a rebuild
//!   pylon runtime latest          # newest published image

use pylon_kernel::ExitCode;
use serde::Deserialize;

use crate::cloud_client::{post_json, require_credentials, Credentials};
use crate::output;
use crate::project_context::resolve_project_slug;

#[derive(Deserialize)]
struct ProjectIdResponse {
    id: String,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    // Must drop flag VALUES, not just flags: `pylon runtime --project pad`
    // read "pad" as the version and tried to upgrade the runtime to a
    // release called "pad".
    let positional = crate::commands::args::collect_positional(args, "runtime");
    let creds = match require_credentials() {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&e);
            eprintln!("  Run: pylon login");
            return ExitCode::Usage;
        }
    };
    let project_slug = match resolve_project_slug(args, &creds, json_mode) {
        Ok(s) => s,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Usage;
        }
    };
    let project_id = match resolve_project_id(&creds, &project_slug) {
        Ok(id) => id,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };

    match positional.first().copied() {
        None | Some("show") | Some("status") => run_show(&creds, &project_id, json_mode),
        Some(version) => run_upgrade(&creds, &project_id, version, json_mode),
    }
}

fn run_show(creds: &Credentials, project_id: &str, json_mode: bool) -> ExitCode {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
    }
    #[derive(Deserialize)]
    struct Out {
        image: Option<String>,
        version: Option<String>,
    }
    let out: Out = match post_json(
        creds,
        "/api/fn/getProjectRuntimeImage",
        &Args { project_id },
    ) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "image": out.image, "version": out.version,
            }))
            .unwrap_or_default()
        );
        return ExitCode::Ok;
    }
    match &out.version {
        Some(v) => println!("Runtime: pylon {v}"),
        None => println!("Runtime: unknown (no live machine yet)"),
    }
    // The declared version in package.json is what a deploy boots on, so
    // point at the real control instead of at a manual bump. A mismatch
    // here means the app hasn't been deployed since that pin changed.
    match declared_sdk_version() {
        Some(declared) if out.version.as_deref() == Some(declared.as_str()) => {
            eprintln!("  Matches @pylonsync/* in package.json.");
        }
        Some(declared) => {
            eprintln!("  package.json declares {declared} — deploy to move the runtime there.");
        }
        None => {
            eprintln!("  Set by @pylonsync/* in your package.json on the next deploy.");
        }
    }
    ExitCode::Ok
}

/// The `@pylonsync/*` version this project's package.json asks for, if it
/// declares a plain one. Ranges (`^0.4.0`) resolve at install time, so a
/// caret is reported without its prefix only when it names an exact
/// release — anything fuzzier isn't something to compare against.
fn declared_sdk_version() -> Option<String> {
    let text = std::fs::read_to_string("package.json").ok()?;
    declared_sdk_version_in(&text)
}

/// Split out so the parsing is testable without `chdir` — cargo runs tests
/// in threads, and changing the process-wide cwd races every other test.
pub(crate) fn declared_sdk_version_in(package_json: &str) -> Option<String> {
    let pkg: serde_json::Value = serde_json::from_str(package_json).ok()?;
    for field in ["dependencies", "devDependencies"] {
        let Some(deps) = pkg.get(field).and_then(|d| d.as_object()) else {
            continue;
        };
        for name in ["@pylonsync/functions", "@pylonsync/sdk", "@pylonsync/react"] {
            let Some(spec) = deps.get(name).and_then(|v| v.as_str()) else {
                continue;
            };
            let cleaned = spec.trim_start_matches(['^', '~', '=', 'v']);
            let is_exact = cleaned.split('.').count() == 3
                && cleaned
                    .split('.')
                    .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
            if is_exact {
                return Some(cleaned.to_string());
            }
        }
    }
    None
}

fn run_upgrade(creds: &Credentials, project_id: &str, version: &str, json_mode: bool) -> ExitCode {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        #[serde(rename = "projectId")]
        project_id: &'a str,
        version: &'a str,
    }
    #[derive(Deserialize)]
    struct Out {
        image: String,
        flipped: u32,
    }
    if !json_mode {
        println!("Upgrading runtime to pylon {version}…");
    }
    let out: Out = match post_json(
        creds,
        "/api/fn/upgradeProjectRuntime",
        &Args {
            project_id,
            version,
        },
    ) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };
    if json_mode {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "ok": true, "image": out.image, "flipped": out.flipped,
            }))
            .unwrap_or_default()
        );
        return ExitCode::Ok;
    }
    println!(
        "✓ Runtime → {} ({} machine{} rebooting on the new binary)",
        out.image,
        out.flipped,
        if out.flipped == 1 { "" } else { "s" }
    );
    ExitCode::Ok
}

fn resolve_project_id(creds: &Credentials, slug: &str) -> Result<String, String> {
    #[derive(serde::Serialize)]
    struct Args<'a> {
        slug: &'a str,
    }
    let proj: ProjectIdResponse = post_json(creds, "/api/fn/getProjectForCli", &Args { slug })
        .map_err(|e| format!("Could not resolve project \"{slug}\": {e}"))?;
    Ok(proj.id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_exact_declared_version() {
        let out = declared_sdk_version_in(r#"{"dependencies":{"@pylonsync/functions":"0.4.7"}}"#);
        assert_eq!(out.as_deref(), Some("0.4.7"));
    }

    #[test]
    fn strips_a_caret_when_it_still_names_one_release() {
        let out = declared_sdk_version_in(r#"{"dependencies":{"@pylonsync/sdk":"^0.4.7"}}"#);
        assert_eq!(out.as_deref(), Some("0.4.7"));
    }

    #[test]
    fn ignores_specs_that_dont_name_a_release() {
        // "latest", a git url, or a workspace spec resolve at install time —
        // there is nothing to compare the running runtime against.
        for spec in [
            "latest",
            "workspace:*",
            "*",
            "0.4",
            "github:pylonsync/pylon",
        ] {
            let json = format!(r#"{{"dependencies":{{"@pylonsync/sdk":"{spec}"}}}}"#);
            assert_eq!(
                declared_sdk_version_in(&json),
                None,
                "spec {spec:?} should not resolve to a version"
            );
        }
    }

    #[test]
    fn prefers_functions_the_package_that_speaks_the_protocol() {
        // A mismatch in @pylonsync/functions is the one that produces
        // confusing failures — it talks to the Rust host over stdio.
        let out = declared_sdk_version_in(
            r#"{"dependencies":{"@pylonsync/react":"0.4.5","@pylonsync/functions":"0.4.7"}}"#,
        );
        assert_eq!(out.as_deref(), Some("0.4.7"));
    }

    #[test]
    fn finds_it_in_dev_dependencies_too() {
        let out = declared_sdk_version_in(r#"{"devDependencies":{"@pylonsync/sdk":"0.4.6"}}"#);
        assert_eq!(out.as_deref(), Some("0.4.6"));
    }

    #[test]
    fn a_package_json_without_pylon_deps_is_not_an_error() {
        assert_eq!(declared_sdk_version_in(r#"{"name":"x"}"#), None);
        assert_eq!(declared_sdk_version_in("not json"), None);
    }
}
