//! `pylon login` — authenticate against Pylon Cloud (or a self-hosted
//! Pylon Cloud install via `PYLON_CLOUD_URL`).
//!
//! Flow:
//! 1. Print a URL to the cloud's "Generate a CLI token" page.
//! 2. Try to open it in the user's browser via `open` (macOS) / `xdg-open`
//!    (Linux) / `start` (Windows). Failure is non-fatal — the URL is
//!    already printed.
//! 3. Read the pasted token from stdin.
//! 4. Validate it by hitting `getMe` on the cloud — proves the token
//!    works AND surfaces which user account it belongs to.
//! 5. Persist to `~/.config/pylon/credentials.json` (mode 0600).
//!
//! No OAuth callback dance — paste-token is what every CLI of this
//! shape (Fly, Vercel, Doppler, Wrangler) starts with, and it works
//! over SSH / headless / corp-firewalled environments where browser
//! callbacks don't.

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

    // Interactive fallback for humans at a terminal. Agents should
    // never reach this — they'll get a paste prompt that hangs.
    let token_url = format!(
        "{}/dashboard/account/cli-tokens",
        crate::cloud_client::dashboard_url()
    );

    if !json_mode {
        println!();
        println!("→ Sign in at: {token_url}");
        println!("  Click \"Create CLI token\", copy the value, and paste it below.");
        println!("  Non-interactive: pylon login --token <token>");
        println!("                   PYLON_CLI_TOKEN=<token> pylon login");
        println!("                   echo <token> | pylon login --token-stdin");
        println!();
    }

    // Best-effort browser launch. Errors are silent — the URL is
    // already printed for headless / SSH callers.
    let _ = open_browser(&token_url);

    print!("Paste token: ");
    let _ = io::stdout().flush();

    let stdin = io::stdin();
    let mut line = String::new();
    if stdin.lock().read_line(&mut line).is_err() {
        output::print_error("Failed to read token from stdin.");
        return ExitCode::Usage;
    }
    let token = line.trim().to_string();
    if token.is_empty() {
        output::print_error("No token provided. Aborting.");
        return ExitCode::Usage;
    }

    // Validate before persisting — better to fail loudly here than to
    // write a bad credential to disk and have every subsequent command
    // hit 401.
    let email = match validate_token(&cloud, &token) {
        Ok(email) => email,
        Err(e) => {
            output::print_error(&format!("Token did not validate: {e}"));
            return ExitCode::Usage;
        }
    };

    let creds = Credentials {
        cloud_url: cloud.clone(),
        token,
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
        });
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
    } else {
        println!();
        println!("✓ Signed in as {email}");
        println!("  Cloud: {cloud}");
        println!("  You can now run `pylon deploy --target cloud` from any Pylon project.");
    }
    ExitCode::Ok
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
