//! `pylon admin` — manage the Studio operator accounts.
//!
//! Studio requires a signed-in admin. For apps whose admins are ordinary users
//! that's `PYLON_ADMIN_EMAILS` or `auth.user.adminField`. For everything else —
//! an API-only backend, a brand-new project, an app whose auth is OAuth-only —
//! operators are the way in, and this is how they're created.
//!
//! ```text
//! pylon admin create <username> [--password <pw>]
//! pylon admin passwd <username> [--password <pw>]
//! pylon admin list
//! pylon admin rm <username>
//! ```
//!
//! # Authorization
//!
//! These drive `/admin/operators` on a running Pylon, authorized by
//! `PYLON_ADMIN_TOKEN` — the same credential that already authorizes every
//! other `/admin/*` route. No new global key was invented, deliberately:
//! anyone holding the admin token can already read and write every row through
//! those endpoints, so letting them mint an operator hands out no authority
//! they lacked. What changes is that the token stops being something a human
//! carries into a browser, each operator is separately revocable, and actions
//! get a name attached.
//!
//! `--url` defaults to the local dev server. Point it at a deployment to
//! bootstrap that one.

use crate::output::print_error;
use pylon_kernel::ExitCode;

const DEFAULT_URL: &str = "http://127.0.0.1:4321";

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    if args.iter().any(|a| a == "--help") || args.len() < 2 {
        print_usage();
        return if args.len() < 2 {
            ExitCode::Usage
        } else {
            ExitCode::Ok
        };
    }

    let sub = args[1].as_str();
    let positional: Vec<&String> = args[2..].iter().filter(|a| !a.starts_with("--")).collect();
    let base = flag(args, "--url").unwrap_or_else(|| DEFAULT_URL.to_string());

    let token = match std::env::var("PYLON_ADMIN_TOKEN") {
        Ok(t) if !t.trim().is_empty() => t,
        _ => {
            print_error(
                "PYLON_ADMIN_TOKEN is not set.\n\
                 Operator management authorizes with the same token as the rest of \
                 /admin/*. Export the value this Pylon runs with:\n\n  \
                 export PYLON_ADMIN_TOKEN=…",
            );
            return ExitCode::Error;
        }
    };

    match sub {
        "create" | "passwd" => {
            let Some(username) = positional.first() else {
                print_error(&format!(
                    "`pylon admin {sub}` needs a username\nUsage: pylon admin {sub} <username>"
                ));
                return ExitCode::Usage;
            };
            let password = match resolve_password(args) {
                Ok(p) => p,
                Err(code) => return code,
            };
            let (path, body) = if sub == "create" {
                (
                    "/admin/operators".to_string(),
                    serde_json::json!({ "username": username, "password": password }),
                )
            } else {
                (
                    format!("/admin/operators/{username}/password"),
                    serde_json::json!({ "password": password }),
                )
            };
            match post(&base, &path, &token, &body) {
                Ok(_) => {
                    if json_mode {
                        println!("{}", serde_json::json!({"ok": true, "username": username}));
                    } else if sub == "create" {
                        println!("Created operator '{username}'. Sign in at {base}/studio/login");
                    } else {
                        println!("Password updated for '{username}'.");
                    }
                    ExitCode::Ok
                }
                Err(e) => {
                    print_error(&e);
                    ExitCode::Error
                }
            }
        }
        "list" => match get(&base, "/admin/operators", &token) {
            Ok(v) => {
                let ops = v["operators"].as_array().cloned().unwrap_or_default();
                if json_mode {
                    println!("{}", serde_json::json!({ "operators": ops }));
                } else if ops.is_empty() {
                    println!("No operators. Create one with `pylon admin create <username>`.");
                } else {
                    for o in &ops {
                        println!("{}", o["username"].as_str().unwrap_or("?"));
                    }
                }
                ExitCode::Ok
            }
            Err(e) => {
                print_error(&e);
                ExitCode::Error
            }
        },
        "rm" => {
            let Some(username) = positional.first() else {
                print_error("`pylon admin rm` needs a username\nUsage: pylon admin rm <username>");
                return ExitCode::Usage;
            };
            match delete(&base, &format!("/admin/operators/{username}"), &token) {
                Ok(_) => {
                    // The server revokes the operator's live sessions as part of
                    // the delete; say so, because "removed" that leaves a working
                    // cookie behind is not what anyone means by it.
                    println!("Removed operator '{username}' and revoked its sessions.");
                    ExitCode::Ok
                }
                Err(e) => {
                    print_error(&e);
                    ExitCode::Error
                }
            }
        }
        other => {
            print_error(&format!(
                "unknown subcommand `{other}`\nRun `pylon admin --help` for usage."
            ));
            ExitCode::Usage
        }
    }
}

/// Password from `--password`, else `PYLON_ADMIN_PASSWORD`, else an interactive
/// prompt.
///
/// The prompt is the default on purpose: a password passed as an argument ends
/// up in shell history and in the process list, where any other user on the box
/// can read it with `ps`. The flag exists for scripted provisioning, which has
/// nowhere better to put it.
fn resolve_password(args: &[String]) -> Result<String, ExitCode> {
    if let Some(p) = flag(args, "--password") {
        return Ok(p);
    }
    if let Ok(p) = std::env::var("PYLON_ADMIN_PASSWORD") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    match rpassword::prompt_password("Password: ") {
        Ok(p) if !p.is_empty() => match rpassword::prompt_password("Confirm password: ") {
            Ok(c) if c == p => Ok(p),
            Ok(_) => {
                print_error("passwords did not match");
                Err(ExitCode::Usage)
            }
            Err(e) => {
                print_error(&format!("could not read password: {e}"));
                Err(ExitCode::Error)
            }
        },
        Ok(_) => {
            print_error("password must not be empty");
            Err(ExitCode::Usage)
        }
        Err(e) => {
            print_error(&format!(
                "could not read password: {e}\n\
                 In a non-interactive shell, pass --password or set PYLON_ADMIN_PASSWORD."
            ));
            Err(ExitCode::Error)
        }
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .filter(|v| !v.starts_with("--"))
        .cloned()
}

fn post(
    base: &str,
    path: &str,
    token: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    send(ureq::post(&format!("{base}{path}")), token, Some(body))
}

fn get(base: &str, path: &str, token: &str) -> Result<serde_json::Value, String> {
    send(ureq::get(&format!("{base}{path}")), token, None)
}

fn delete(base: &str, path: &str, token: &str) -> Result<serde_json::Value, String> {
    send(ureq::delete(&format!("{base}{path}")), token, None)
}

fn send(
    req: ureq::Request,
    token: &str,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let req = req.set("Authorization", &format!("Bearer {token}"));
    let result = match body {
        Some(b) => req.send_json(b.clone()),
        None => req.call(),
    };
    match result {
        Ok(resp) => Ok(resp
            .into_json::<serde_json::Value>()
            .unwrap_or(serde_json::Value::Null)),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp
                .into_json::<serde_json::Value>()
                .unwrap_or(serde_json::Value::Null);
            let msg = body["error"]["message"]
                .as_str()
                .or_else(|| body["message"].as_str())
                .unwrap_or("request failed")
                .to_string();
            Err(match code {
                401 => format!(
                    "{msg} (HTTP 401). Check that PYLON_ADMIN_TOKEN matches the value this Pylon runs with."
                ),
                _ => format!("{msg} (HTTP {code})"),
            })
        }
        Err(e) => Err(format!(
            "could not reach the Pylon: {e}\nIs it running? Pass --url to point at a different one."
        )),
    }
}

fn print_usage() {
    println!(
        r#"pylon admin — manage Studio operator accounts

USAGE
  pylon admin create <username> [--password <pw>]
  pylon admin passwd <username> [--password <pw>]
  pylon admin list
  pylon admin rm <username>

OPTIONS
  --url <base>        Pylon to manage (default {DEFAULT_URL})
  --password <pw>     Password, instead of the interactive prompt.
                      Prefer the prompt: an argument is visible in shell
                      history and to `ps`. PYLON_ADMIN_PASSWORD also works.

AUTHORIZATION
  Requires PYLON_ADMIN_TOKEN, the same credential that authorizes every
  other /admin/* route. Operators sign in at /studio/login.

WHY OPERATORS
  Studio needs a signed-in admin. If your app's admins are ordinary users,
  set PYLON_ADMIN_EMAILS or auth.user.adminField instead and sign in through
  the app. Operators are for deployments whose operators aren't app users —
  including any backend with no users at all."#
    );
}
