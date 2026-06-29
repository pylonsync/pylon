//! `pylon diagnostics` — read the dev server's diagnostics ring.
//!
//! The machine-readable side of the dev HUD, for agents (and humans) without a
//! browser. Fetches `GET /_pylon/dev/diagnostics` from a running `pylon dev` and
//! prints each recent SSR render's cache verdict + the reason it is/isn't cached
//! + render time. `--json` emits the raw payload for programmatic consumers.
//!
//! Usage:
//!   pylon diagnostics [--port 4321] [--json]

use crate::output::print_error;
use pylon_kernel::ExitCode;

const DEFAULT_PORT: u16 = 4321;

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Usage: pylon diagnostics [--port <port>] [--json]");
        println!();
        println!("  Reads the running dev server's SSR diagnostics ring (cache");
        println!("  verdict + reason + render timing per route). Run `pylon dev`");
        println!("  first. --json prints the raw payload.");
        return ExitCode::Ok;
    }

    let port = parse_flag_u16(args, "--port").unwrap_or(DEFAULT_PORT);
    let json = json_mode || args.iter().any(|a| a == "--json");
    let url = format!("http://127.0.0.1:{port}/_pylon/dev/diagnostics");

    let body = match ureq::get(&url)
        .timeout(std::time::Duration::from_secs(3))
        .call()
    {
        Ok(resp) => match resp.into_string() {
            Ok(s) => s,
            Err(e) => {
                print_error(&format!("failed to read diagnostics response: {e}"));
                return ExitCode::Error;
            }
        },
        Err(ureq::Error::Status(404, _)) => {
            print_error(
                "diagnostics endpoint returned 404 — the server isn't in dev mode \
                 (diagnostics are dev-only).",
            );
            return ExitCode::Unavailable;
        }
        Err(_) => {
            print_error(&format!(
                "no Pylon dev server on :{port} — start one with `pylon dev` \
                 (or pass --port)."
            ));
            return ExitCode::Unavailable;
        }
    };

    if json {
        println!("{body}");
        return ExitCode::Ok;
    }

    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            print_error(&format!("malformed diagnostics payload: {e}"));
            return ExitCode::Error;
        }
    };
    let ssr = parsed.get("ssr").and_then(|v| v.as_array());
    let Some(ssr) = ssr else {
        println!("No SSR renders recorded yet — hit a page in the browser first.");
        return ExitCode::Ok;
    };
    if ssr.is_empty() {
        println!("No SSR renders recorded yet — hit a page in the browser first.");
        return ExitCode::Ok;
    }

    println!("Recent SSR renders (newest last):");
    println!();
    for ev in ssr {
        let route = ev.get("route").and_then(|v| v.as_str()).unwrap_or("?");
        let verdict = ev.get("verdict").and_then(|v| v.as_str()).unwrap_or("?");
        let reason = ev.get("reason").and_then(|v| v.as_str()).unwrap_or("");
        let ms = ev.get("render_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let status = ev.get("status").and_then(|v| v.as_u64()).unwrap_or(0);
        let secs = ev.get("secs").and_then(|v| v.as_u64());
        let ttl = secs.map(|s| format!(" {s}s")).unwrap_or_default();
        let reason_suffix = if reason.is_empty() {
            String::new()
        } else {
            format!("  — {reason}")
        };
        println!("  {status}  {verdict}{ttl}  {route}  ·  {ms:.1}ms{reason_suffix}");
    }
    ExitCode::Ok
}

fn parse_flag_u16(args: &[String], flag: &str) -> Option<u16> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
}
