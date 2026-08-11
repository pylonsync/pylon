//! `pylon whoami` — which account, which cloud, which project.
//!
//! The first thing to run when a command targets the wrong thing.
//! Three questions get answered together because they fail together:
//! a stale token, a `PYLON_CLOUD_URL` pointing at staging, and a
//! `.pylon/project` inherited from a parent directory all present as
//! "why did that go somewhere else?"
//!
//! Reports where the project slug came from, since the resolution
//! order (flag → env → context file → machine-global default) is
//! exactly what makes the answer surprising.

use pylon_kernel::ExitCode;
use serde::Deserialize;

use crate::cloud_client::{load_credentials, post_json, Credentials};
use crate::output;
use crate::project_context::{resolve_project_with_source, ProjectSource};

#[derive(Deserialize)]
struct Me {
    email: String,
    #[serde(rename = "displayName", default)]
    display_name: String,
    #[serde(rename = "isAdmin", default)]
    is_admin: bool,
}

#[derive(Deserialize)]
struct ProjectEntry {
    slug: String,
    #[serde(rename = "orgSlug", default)]
    org_slug: Option<String>,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("pylon whoami — show the signed-in account, cloud, and active project");
        println!();
        println!("USAGE");
        println!("  pylon whoami [--json]");
        return ExitCode::Ok;
    }

    let creds = match load_credentials() {
        Ok(Some(c)) => c,
        Ok(None) => {
            if json_mode {
                println!("{}", serde_json::json!({"loggedIn": false}));
            } else {
                println!("Not logged in.");
                println!("  Run: pylon login");
            }
            // Usage, not Error: "not logged in" is a state to report,
            // and `pylon whoami || pylon login` should read cleanly.
            return ExitCode::Usage;
        }
        Err(e) => {
            output::print_error(&format!("Failed to read credentials: {e}"));
            return ExitCode::Error;
        }
    };

    // Round-trip the token rather than trusting the cached email on
    // disk. A revoked or expired token is the single most common
    // reason a cloud command misbehaves, and the cached copy would
    // happily report a login that no longer works.
    let me: Me = match post_json(&creds, "/api/fn/getMe", &()) {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&format!("Token rejected by {}: {e}", creds.cloud_url));
            eprintln!("  Run: pylon login");
            return ExitCode::Error;
        }
    };

    let project = resolve_project_with_source(args);
    let lookup = project
        .as_ref()
        .map(|(slug, _)| lookup_project(&creds, slug))
        .unwrap_or(ProjectLookup::Unavailable);

    if json_mode {
        let (org, visible) = match &lookup {
            ProjectLookup::Found { org } => (org.clone(), Some(true)),
            ProjectLookup::NotVisible => (None, Some(false)),
            ProjectLookup::Unavailable => (None, None),
        };
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "loggedIn": true,
                "email": me.email,
                "displayName": me.display_name,
                "isAdmin": me.is_admin,
                "cloudUrl": creds.cloud_url,
                "project": project.as_ref().map(|(slug, source)| serde_json::json!({
                    "slug": slug,
                    "org": org,
                    // null when the project list couldn't be fetched —
                    // "unknown" is not the same as "not yours".
                    "visible": visible,
                    "source": source_key(*source),
                })),
            }))
            .unwrap_or_default()
        );
        return ExitCode::Ok;
    }

    if me.display_name.is_empty() {
        println!("{}", me.email);
    } else {
        println!("{} ({})", me.email, me.display_name);
    }
    if me.is_admin {
        println!("  role      platform admin");
    }
    println!("  cloud     {}", creds.cloud_url);
    match &project {
        Some((slug, source)) => {
            let qualified = match &lookup {
                ProjectLookup::Found { org: Some(o) } => format!("{o}/{slug}"),
                _ => slug.clone(),
            };
            println!("  project   {qualified}  ({})", source_label(*source));
            // A slug that resolves locally but names nothing on this
            // account is the failure this command exists to catch: a
            // deleted project, a renamed slug, or a context file
            // inherited from a parent directory that belongs to a
            // different account. Every cloud command will fail against
            // it with a less obvious error.
            if matches!(lookup, ProjectLookup::NotVisible) {
                println!("            ! no project \"{slug}\" on this account — stale context");
                println!("              Fix: pylon projects use <slug>");
            }
        }
        None => {
            println!("  project   none — run `pylon projects use <slug>`");
        }
    }
    ExitCode::Ok
}

/// What the caller's project list says about the resolved slug.
enum ProjectLookup {
    /// The slug names a project this account can see.
    Found { org: Option<String> },
    /// The list came back and the slug wasn't in it.
    NotVisible,
    /// The list couldn't be fetched. Not the same as NotVisible —
    /// reporting "stale context" on a network blip would send someone
    /// off relinking a project that was fine.
    Unavailable,
}

fn lookup_project(creds: &Credentials, slug: &str) -> ProjectLookup {
    let projects: Vec<ProjectEntry> = match post_json(creds, "/api/fn/listMyProjectsForCli", &()) {
        Ok(p) => p,
        Err(_) => return ProjectLookup::Unavailable,
    };
    match projects.into_iter().find(|p| p.slug == slug) {
        Some(p) => ProjectLookup::Found { org: p.org_slug },
        None => ProjectLookup::NotVisible,
    }
}

fn source_label(source: ProjectSource) -> &'static str {
    match source {
        ProjectSource::Flag => "from --project",
        ProjectSource::Env => "from $PYLON_PROJECT",
        ProjectSource::ContextFile => "from .pylon/project",
        ProjectSource::GlobalDefault => "machine default — not linked to this directory",
    }
}

fn source_key(source: ProjectSource) -> &'static str {
    match source {
        ProjectSource::Flag => "flag",
        ProjectSource::Env => "env",
        ProjectSource::ContextFile => "context-file",
        ProjectSource::GlobalDefault => "global-default",
    }
}
