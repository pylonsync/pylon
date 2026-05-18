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

pub fn run(_args: &[String], json_mode: bool) -> ExitCode {
    let cloud = cloud_url();
    let token_url = format!(
        "{}/dashboard/account/cli-tokens",
        cloud.trim_end_matches('/')
    );

    if !json_mode {
        println!();
        println!("→ Sign in at: {token_url}");
        println!("  Click \"Create CLI token\", copy the value, and paste it below.");
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
