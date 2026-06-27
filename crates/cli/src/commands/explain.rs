//! `pylon explain <code>` — look up a Pylon error code and print
//! what it means + how to fix it. Agent-targeted: every code Pylon
//! emits at any layer (router, sync, auth, policy, deploy) has an
//! entry here so a CLI invocation that just got back
//! `{"error":{"code":"FOO"}}` can run `pylon explain FOO --json` and
//! know exactly what to do next without a doc round-trip.
//!
//! Backward-compat: if the arg looks like a path (contains `/`,
//! starts with `.`, or ends with `.json`), falls through to the old
//! behavior — print a summary of the manifest at that path.

use pylon_kernel::ExitCode;

use crate::manifest::load_manifest;
use crate::output::{print_diagnostics, print_json};

struct CodeEntry {
    code: &'static str,
    summary: &'static str,
    fix: &'static str,
}

const CODES: &[CodeEntry] = &[
    // --- Auth ---
    CodeEntry {
        code: "UNAUTHENTICATED",
        summary: "No valid session token on the request.",
        fix: "Pass Authorization: Bearer <token>, send cookies via credentials: \"include\", or call /api/auth/login to mint one.",
    },
    CodeEntry {
        code: "AUTH_REQUIRED",
        summary: "Same as UNAUTHENTICATED — the endpoint requires a signed-in user.",
        fix: "Sign in via /api/auth/login / /api/auth/magic/verify / OAuth, then retry with the returned token.",
    },
    CodeEntry {
        code: "FORBIDDEN",
        summary: "Authenticated, but the caller's role / policy doesn't allow this action.",
        fix: "Check the entity's policy or the function's `auth` field. The token is valid; the user just isn't allowed.",
    },
    CodeEntry {
        code: "INVALID_TOKEN",
        summary: "Token failed verification (expired, signed with a different secret, or malformed).",
        fix: "Run `pylon login` again, or refresh via /api/auth/refresh. Check PYLON_JWT_SECRET on the server hasn't rotated.",
    },
    CodeEntry {
        code: "INVALID_CREDENTIALS",
        summary: "Wrong email/password on /api/auth/password/login.",
        fix: "Reset the password via /api/auth/password/reset/request, or sign in with magic link.",
    },
    CodeEntry {
        code: "RATE_LIMITED",
        summary: "Hit the per-IP or per-user request budget.",
        fix: "Back off (the response carries Retry-After). Increase PYLON_RATE_LIMIT_MAX(_AUTHED) if the limit is too tight.",
    },
    CodeEntry {
        code: "API_KEY_AUTH_FORBIDDEN",
        summary: "This endpoint refuses API-key auth (sessions only).",
        fix: "Use a real session token. API keys are limited to data-plane reads/writes; auth surface needs a session.",
    },
    CodeEntry {
        code: "NOT_A_MEMBER",
        summary: "select-org / org action rejected — the user isn't in the target org.",
        fix: "Get invited to the org via /api/auth/orgs/<orgId>/invites, or pick a different org.",
    },
    CodeEntry {
        code: "LAST_OWNER",
        summary: "Refused to demote / remove the only remaining org owner.",
        fix: "Promote someone else to owner first, then re-run.",
    },

    // --- Validation / shape ---
    CodeEntry {
        code: "INVALID_REQUEST",
        summary: "Body shape didn't match the function's args schema.",
        fix: "Re-check the function definition's v.* args. The cloud response body usually shows the missing/wrong fields.",
    },
    CodeEntry {
        code: "INVALID_JSON",
        summary: "Body wasn't valid JSON.",
        fix: "Verify the request body parses with `JSON.parse` — typical cause is a stray newline or missing Content-Type: application/json.",
    },
    CodeEntry {
        code: "INVALID_ARGS",
        summary: "Args don't conform to the function's declared types.",
        fix: "Compare the request body to the action's `args: { ... }` declaration. Check field names AND types.",
    },
    CodeEntry {
        code: "INVALID_HOSTNAME",
        summary: "Custom-domain hostname failed DNS-name validation.",
        fix: "Use a lowercase, RFC-compliant hostname (e.g. api.acme.com). No protocol, no path, no port.",
    },
    CodeEntry {
        code: "HOSTNAME_TAKEN",
        summary: "Another Pylon project already claimed this hostname.",
        fix: "Pick a different hostname, or release it from the other project first.",
    },

    // --- Data / policy ---
    CodeEntry {
        code: "NOT_FOUND",
        summary: "Row, entity, or route doesn't exist.",
        fix: "Verify the id / slug / entity name. For policy-filtered reads, check whether the caller is allowed to see this row.",
    },
    CodeEntry {
        code: "CONFLICT",
        summary: "Unique constraint hit (duplicate email, slug, etc.) or version mismatch.",
        fix: "Re-fetch the row to get the current state, then retry with the correct expected version.",
    },
    CodeEntry {
        code: "UPGRADE_APPROVAL_REQUIRED",
        summary: "Pylon Cloud paywalled this resize behind admin approval.",
        fix: "Submit an upgrade request from the dashboard — admin reviews and approves before the resize lands.",
    },

    // --- Sync ---
    CodeEntry {
        code: "RESYNC_REQUIRED",
        summary: "Server returned 410 — your cursor is from a previous server lifetime or fell off the retention window.",
        fix: "The SyncEngine handles this automatically: resets the local replica and re-pulls from seq=0. Manual fix: clear IndexedDB.",
    },
    CodeEntry {
        code: "PUSH_REJECTED",
        summary: "Mutation pushed but the server refused (policy, validation, version).",
        fix: "Inspect the response body; the engine surfaces the per-op error.code so you can map it to a UI message.",
    },

    // --- Connections / OAuth ---
    CodeEntry {
        code: "CONNECTION_UNKNOWN",
        summary: "No defineConnection({name:...}) matches the requested name.",
        fix: "Add a defineConnection entry to your manifest, or fix the typo in `<ConnectAccount name=>`.",
    },
    CodeEntry {
        code: "PROVIDER_NOT_CONFIGURED",
        summary: "Connection exists but the OAuth provider's CLIENT_ID/SECRET env vars aren't set.",
        fix: "Set PYLON_OAUTH_<PROVIDER>_CLIENT_ID + _CLIENT_SECRET on the server, then redeploy.",
    },
    CodeEntry {
        code: "ENCRYPTION_REQUIRED",
        summary: "Connection storage needs PYLON_ENCRYPTION_KEY (or field.encrypted() use needs it).",
        fix: "Set PYLON_ENCRYPTION_KEY (32 bytes base64) on the server. See `pylon env` for the canonical name.",
    },

    // --- Cloud deploy ---
    CodeEntry {
        code: "DEPLOY_FAILED",
        summary: "pylon-cloud rejected the deploy. Body carries the underlying reason.",
        fix: "Read the embedded message. Common: machine missing flyMachineId (control-plane state), Fly outage, image pull failure.",
    },
    CodeEntry {
        code: "NO_MACHINES",
        summary: "Project has no Fly Machines provisioned yet.",
        fix: "Wait for the initial provision to complete (it's async), then re-run `pylon deploy`.",
    },
    CodeEntry {
        code: "CLOUD_MISCONFIGURED",
        summary: "Pylon-cloud control plane is missing an env var (FLY_API_TOKEN, etc.).",
        fix: "Operator concern — set the missing var on the control-plane Fly Machine and redeploy it.",
    },
];

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let arg = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "explain")
        .next()
        .map(|s| s.as_str());

    let Some(arg) = arg else {
        if json_mode {
            // List all codes when no arg — agents can pipe through jq.
            let all: Vec<_> = CODES
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "code": c.code,
                        "summary": c.summary,
                        "fix": c.fix,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string(&all).unwrap_or("[]".into()));
            return ExitCode::Ok;
        }
        eprintln!("Usage: pylon explain <ERROR_CODE>");
        eprintln!();
        eprintln!("Known codes:");
        for c in CODES {
            eprintln!("  {}", c.code);
        }
        eprintln!();
        eprintln!("Or pass a manifest path to inspect that manifest instead.");
        return ExitCode::Usage;
    };

    // Path-shaped argument → fall through to manifest inspect for
    // backward compatibility.
    let looks_like_path = arg.contains('/') || arg.starts_with('.') || arg.ends_with(".json");
    if looks_like_path {
        return inspect_manifest(arg, json_mode);
    }

    // Code lookup is case-insensitive — agents see codes in JSON
    // bodies and might paste them with mixed case.
    let upper = arg.to_ascii_uppercase();
    let entry = CODES.iter().find(|c| c.code == upper);
    match entry {
        Some(c) => {
            if json_mode {
                let out = serde_json::json!({
                    "code": c.code,
                    "summary": c.summary,
                    "fix": c.fix,
                });
                println!("{}", serde_json::to_string(&out).unwrap_or_default());
            } else {
                println!("{}", c.code);
                println!();
                println!("  {}", c.summary);
                println!();
                println!("  Fix: {}", c.fix);
            }
            ExitCode::Ok
        }
        None => {
            if json_mode {
                let out = serde_json::json!({
                    "code": upper,
                    "known": false,
                    "hint": "Run `pylon explain --json` for the full list.",
                });
                println!("{}", serde_json::to_string(&out).unwrap_or_default());
            } else {
                eprintln!("Unknown error code: {upper}");
                eprintln!("Run `pylon explain` (no arg) to see the full list.");
            }
            ExitCode::Usage
        }
    }
}

fn inspect_manifest(path: &str, json_mode: bool) -> ExitCode {
    let manifest = match load_manifest(path) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    if json_mode {
        print_json(&manifest);
    } else {
        println!(
            "App: {} v{} (manifest v{})",
            manifest.name, manifest.version, manifest.manifest_version
        );
        println!();
        println!("Entities:");
        for entity in &manifest.entities {
            println!("  {}", entity.name);
            for field in &entity.fields {
                let mut modifiers = Vec::new();
                if field.optional {
                    modifiers.push("optional");
                }
                if field.unique {
                    modifiers.push("unique");
                }
                let mod_str = if modifiers.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", modifiers.join(", "))
                };
                println!("    {}: {}{}", field.name, field.field_type, mod_str);
            }
            for index in &entity.indexes {
                let unique_str = if index.unique { " [unique]" } else { "" };
                println!(
                    "    [index] {} on ({}){}",
                    index.name,
                    index.fields.join(", "),
                    unique_str
                );
            }
        }
        println!();
        println!("Policies:");
        for policy in &manifest.policies {
            let entity = policy.entity.as_deref().unwrap_or("<all>");
            println!("  {} on {}", policy.name, entity);
        }
    }
    ExitCode::Ok
}
