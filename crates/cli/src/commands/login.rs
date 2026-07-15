//! `pylon login` — authenticate against Pylon Cloud (or a self-hosted
//! Pylon Cloud install via `PYLON_CLOUD_URL`).
//!
//! Default (interactive) flow — device authorization, no manual paste of a
//! long token:
//! 1. Generate a typeable, high-entropy login code locally.
//! 2. Print it and open a bare `{dashboard}/cli` in the browser (the code is
//!    NEVER put in the URL — a link alone must not be able to approve a login).
//! 3. The signed-in user TYPES the code on `/cli` and approves; the browser
//!    mints a token and stashes it under the typed code.
//! 4. The CLI polls `exchangeCliAuthCode({code})` until the token appears,
//!    then persists to `~/.config/pylon/credentials.json` (mode 0600).
//!
//! Signup happens inline: an unauthenticated visitor to `/cli` is bounced
//! through signup/login and lands back on the approve screen.
//!
//! Non-interactive paths (agents / CI) skip the browser entirely:
//!   --code <code>       redeem a one-time code minted in the dashboard
//!   --token <token>     direct
//!   --token-stdin       read the token from stdin
//!   PYLON_CLI_TOKEN=…   env var

use std::io::{self, BufRead, Write};

use pylon_kernel::ExitCode;

use crate::cloud_client::{
    cloud_url, delete_credentials, save_credentials, validate_token, Credentials,
};
use crate::output;

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let cloud = cloud_url();

    // `pylon login --code XXXX-XXXX` — agent-onboarding handoff.
    // Dashboard mints a real token, stashes it behind a one-time
    // 8-char code, shows the code to the user inside a paste-ready
    // Claude Code prompt. The agent then runs THIS path to redeem.
    // Trades the 6-step "click, copy, paste, return" loop for a
    // single autonomous step.
    let code_flag = args
        .windows(2)
        .find(|w| w[0] == "--code")
        .map(|w| w[1].clone())
        .or_else(|| {
            args.iter()
                .find(|a| a.starts_with("--code="))
                .map(|a| a.trim_start_matches("--code=").to_string())
        });

    if let Some(code) = code_flag {
        return run_with_code(&cloud, &code, json_mode);
    }

    // Non-interactive paths for agents / CI:
    //   --token <token>     direct
    //   --token-stdin       read from stdin (single line; works under pipes)
    //   PYLON_CLI_TOKEN=…   env var
    let token_flag = args
        .windows(2)
        .find(|w| w[0] == "--token")
        .map(|w| w[1].clone())
        .or_else(|| {
            args.iter()
                .find(|a| a.starts_with("--token="))
                .map(|a| a.trim_start_matches("--token=").to_string())
        });
    let token_stdin = args.iter().any(|a| a == "--token-stdin");
    let env_token = std::env::var("PYLON_CLI_TOKEN")
        .ok()
        .filter(|s| !s.is_empty());

    if let Some(token) = token_flag.or(env_token) {
        return run_with_token(&cloud, &token, json_mode);
    }
    if token_stdin {
        let stdin = io::stdin();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).is_err() {
            output::print_error("Failed to read token from stdin.");
            return ExitCode::Usage;
        }
        let token = line.trim().to_string();
        if token.is_empty() {
            output::print_error("Empty token on stdin.");
            return ExitCode::Usage;
        }
        return run_with_token(&cloud, &token, json_mode);
    }

    // Interactive default: device-authorization. Opens the browser to the
    // approve page and polls for the token — no paste. Agents/CI never reach
    // here (they hit the token/code/env paths above).
    run_device_flow(&cloud, json_mode)
}

/// Device-authorization sign-in. Prints a login code, opens a bare
/// `{dashboard}/cli`, and polls `exchangeCliAuthCode({code})` until the user
/// types that code in the browser and the approval stashes a token under it.
fn run_device_flow(cloud: &str, json_mode: bool) -> ExitCode {
    use crate::cloud_client::post_json;
    use serde::Deserialize;
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    // `login_code` is the canonical form we poll with: 12 uppercase letters +
    // digits, no separators. We DISPLAY it grouped for readability; the user
    // may type the hyphens or not — the /cli page strips them to the same 12
    // characters before stashing, so poll and stash always match.
    let login_code = generate_login_code();
    let display_code = format!(
        "{}-{}-{}",
        &login_code[0..4],
        &login_code[4..8],
        &login_code[8..12]
    );
    let verify_url = format!("{}/cli", crate::cloud_client::dashboard_url());

    if !json_mode {
        println!();
        println!("→ To sign in, open:  {verify_url}");
        println!("  and enter this code:");
        println!();
        println!("      {display_code}");
        println!();
        println!("  Only type it on {verify_url} — never a code a link or message sent you.");
        println!(
            "  (Headless/CI: pylon login --token <token>  ·  PYLON_CLI_TOKEN=<token> pylon login)"
        );
        println!();
    }
    // Best-effort browser launch — the URL is already printed for headless /
    // SSH callers, who can open it on any device and type the code.
    let _ = open_browser(&verify_url);

    #[derive(serde::Serialize)]
    struct Args<'a> {
        code: &'a str,
    }
    #[derive(Deserialize)]
    struct Out {
        token: String,
        user: UserOut,
    }
    #[derive(Deserialize)]
    struct UserOut {
        email: String,
    }

    // No credentials yet — the code IS the auth for this public endpoint.
    let poll_creds = Credentials {
        cloud_url: cloud.to_string(),
        token: String::new(),
        user_email: None,
    };
    // The code row's TTL is 5 min; give the user that long to approve.
    let deadline = Instant::now() + Duration::from_secs(300);

    if !json_mode {
        print!("  Waiting for approval");
        let _ = io::stdout().flush();
    }
    loop {
        // Check the deadline BEFORE polling so a token can't be accepted after
        // the window, and we never start a poll past it.
        if Instant::now() >= deadline {
            if !json_mode {
                println!();
            }
            output::print_error("Timed out waiting for approval (5 min). Re-run `pylon login`.");
            return ExitCode::Usage;
        }
        match post_json::<_, Out>(
            &poll_creds,
            "/api/fn/exchangeCliAuthCode",
            &Args { code: &login_code },
        ) {
            Ok(resp) => {
                if !json_mode {
                    println!();
                }
                let creds = Credentials {
                    cloud_url: cloud.to_string(),
                    token: resp.token,
                    user_email: Some(resp.user.email.clone()),
                };
                if let Err(e) = save_credentials(&creds) {
                    output::print_error(&format!("Failed to save credentials: {e}"));
                    return ExitCode::Error;
                }
                if json_mode {
                    let out = serde_json::json!({
                        "ok": true,
                        "cloud_url": cloud,
                        "user_email": resp.user.email,
                        "via": "device",
                    });
                    println!("{}", serde_json::to_string(&out).unwrap_or_default());
                } else {
                    println!("✓ Signed in as {}", resp.user.email);
                    println!("  Cloud: {cloud}");
                    println!("  Run `pylon deploy` from any Pylon project to ship it.");
                }
                return ExitCode::Ok;
            }
            Err(msg) => {
                // Terminal outcomes — stop and surface. CODE_INVALID (the row
                // doesn't exist until approval) and transient network blips are
                // NOT terminal: keep polling until the deadline.
                if let Some(err) = device_poll_fatal(&msg) {
                    if !json_mode {
                        println!();
                    }
                    output::print_error(&err);
                    return ExitCode::Error;
                }
                // CODE_INVALID (the row doesn't exist until the user approves)
                // or a transient network blip → keep polling; the deadline is
                // enforced at the top of the loop.
                if !json_mode {
                    print!(".");
                    let _ = io::stdout().flush();
                }
                sleep(Duration::from_secs(2));
            }
        }
    }
}

/// Classify a device-flow poll error. `None` → keep polling (CODE_INVALID
/// means the row doesn't exist until the user approves; network blips are
/// transient). `Some(message)` → stop and surface: consumed/expired/drained
/// codes can never succeed, and an auth rejection (401 AUTH_REQUIRED) means
/// the server's `exchangeCliAuthCode` isn't public — every future poll fails
/// identically, so waiting out the 5-min deadline is a silent hang.
fn device_poll_fatal(msg: &str) -> Option<String> {
    if msg.contains("CODE_USED") || msg.contains("CODE_EXPIRED") || msg.contains("CODE_DRAINED") {
        return Some(format!("Sign-in failed: {msg}"));
    }
    if msg.contains("AUTH_REQUIRED") {
        return Some(format!(
            "Sign-in failed: the cloud rejected the unauthenticated poll — its \
             exchangeCliAuthCode function is not public. Update the Pylon Cloud \
             install (the code, not a session, is the auth for this endpoint). \
             ({msg})"
        ));
    }
    None
}

/// A typeable, high-entropy login code — 12 characters from a 31-char alphabet
/// with no visually-confusable characters (no 0/O, 1/I/L). ~59 bits; combined
/// with the 5-min TTL and the framework's per-IP rate limit, a public poller
/// can't guess it. Uppercase + separator-free so it survives
/// `exchangeCliAuthCode`'s case-insensitive lookup and matches the canonical
/// form the browser stores (which strips any hyphens the user types). The user
/// types this on the /cli page — never a URL param — which is the anti-phishing
/// control: a link alone can't approve a code.
fn generate_login_code() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut rng = rand::thread_rng();
    (0..12)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// `pylon login --token <token>` (or via stdin / env var) — validate
/// the token against the cloud and persist credentials, fully
/// non-interactive. Agents and CI use this; the paste-prompt path is
/// the human fallback.
fn run_with_token(cloud: &str, token: &str, json_mode: bool) -> ExitCode {
    let email = match validate_token(cloud, token) {
        Ok(email) => email,
        Err(e) => {
            output::print_error(&format!("Token did not validate: {e}"));
            return ExitCode::Usage;
        }
    };
    let creds = Credentials {
        cloud_url: cloud.to_string(),
        token: token.to_string(),
        user_email: Some(email.clone()),
    };
    if let Err(e) = save_credentials(&creds) {
        output::print_error(&format!("Failed to save credentials: {e}"));
        return ExitCode::Error;
    }
    if json_mode {
        let out = serde_json::json!({
            "ok": true,
            "cloud_url": cloud,
            "user_email": email,
            "via": "token",
        });
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
    } else {
        println!("✓ Signed in as {email}");
        println!("  Cloud: {cloud}");
    }
    ExitCode::Ok
}

/// `pylon login --code <code>` — exchange a one-time auth code minted
/// in the dashboard for a real API key. Used by the agent-onboarding
/// "Connect Claude Code" flow: dashboard stashes the token behind a
/// short code, the agent redeems it without the token ever appearing
/// in chat history.
fn run_with_code(cloud: &str, code: &str, json_mode: bool) -> ExitCode {
    use crate::cloud_client::post_json;
    use serde::Deserialize;

    #[derive(serde::Serialize)]
    struct Args<'a> {
        code: &'a str,
    }
    #[derive(Deserialize)]
    struct Out {
        token: String,
        #[allow(dead_code)]
        label: String,
        user: UserOut,
    }
    #[derive(Deserialize)]
    struct UserOut {
        email: String,
        #[allow(dead_code)]
        id: String,
    }

    let exchange_creds = Credentials {
        cloud_url: cloud.to_string(),
        token: String::new(),
        user_email: None,
    };
    let resp: Out = match post_json(
        &exchange_creds,
        "/api/fn/exchangeCliAuthCode",
        &Args { code },
    ) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&format!("Exchange failed: {e}"));
            return ExitCode::Usage;
        }
    };

    let creds = Credentials {
        cloud_url: cloud.to_string(),
        token: resp.token,
        user_email: Some(resp.user.email.clone()),
    };
    if let Err(e) = save_credentials(&creds) {
        output::print_error(&format!("Failed to save credentials: {e}"));
        return ExitCode::Error;
    }

    if json_mode {
        let out = serde_json::json!({
            "ok": true,
            "cloud_url": cloud,
            "user_email": resp.user.email,
            "via": "code-exchange",
        });
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
    } else {
        println!("✓ Signed in as {}", resp.user.email);
        println!("  Cloud: {cloud}");
    }
    ExitCode::Ok
}

pub fn run_logout(_args: &[String], json_mode: bool) -> ExitCode {
    match delete_credentials() {
        Ok(true) => {
            if json_mode {
                println!("{{\"ok\":true,\"removed\":true}}");
            } else {
                println!("✓ Signed out. Credentials removed.");
            }
            ExitCode::Ok
        }
        Ok(false) => {
            if json_mode {
                println!("{{\"ok\":true,\"removed\":false}}");
            } else {
                println!("Already signed out — no credentials on disk.");
            }
            ExitCode::Ok
        }
        Err(e) => {
            output::print_error(&format!("Failed to delete credentials: {e}"));
            ExitCode::Error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::device_poll_fatal;

    #[test]
    fn code_invalid_and_network_errors_keep_polling() {
        // The row doesn't exist until the user approves — not fatal.
        assert!(device_poll_fatal(
            "Cloud returned 400: {\"error\":{\"code\":\"CODE_INVALID\",\"message\":\"Unknown auth code.\"}}"
        )
        .is_none());
        // Transient network blip — not fatal.
        assert!(device_poll_fatal("Cloud request failed: connection reset").is_none());
    }

    #[test]
    fn consumed_expired_drained_are_fatal() {
        for code in ["CODE_USED", "CODE_EXPIRED", "CODE_DRAINED"] {
            let msg = format!("Cloud returned 400: {{\"error\":{{\"code\":\"{code}\"}}}}");
            assert!(device_poll_fatal(&msg).is_some(), "{code} must be terminal");
        }
    }

    #[test]
    fn auth_required_is_fatal_not_a_silent_hang() {
        // Regression: a cloud whose exchangeCliAuthCode isn't `auth: "public"`
        // 401s every unauthenticated poll. That's deterministic — the CLI must
        // fail fast with a pointer at the server, not spin dots for 5 minutes
        // while the browser says "You're all set".
        let msg = "Cloud returned 401: {\"error\":{\"code\":\"AUTH_REQUIRED\",\
                   \"message\":\"Function \\\"exchangeCliAuthCode\\\" requires a signed-in user\"}}";
        let err = device_poll_fatal(msg).expect("AUTH_REQUIRED must be terminal");
        assert!(err.contains("exchangeCliAuthCode"));
    }
}

/// Best-effort browser open. macOS `open`, Linux `xdg-open`, Windows
/// `cmd /C start`. Silent failure — the URL is already in the
/// terminal for the user to click.
fn open_browser(url: &str) -> io::Result<()> {
    let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    std::process::Command::new(cmd)
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}
