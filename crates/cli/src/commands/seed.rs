use agentdb_core::ExitCode;

use crate::output::{print_diagnostics, print_json};
use agentdb_core::{Diagnostic, Severity};
use serde::Serialize;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;

#[derive(Serialize)]
struct SeedResult {
    code: &'static str,
    seeded: BTreeMap<String, usize>,
    total: usize,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let file_path = args
        .windows(2)
        .find(|w| w[0] == "--file")
        .map(|w| w[1].as_str())
        .unwrap_or("seed.json");

    let port: u16 = args
        .windows(2)
        .find(|w| w[0] == "--port")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(4000);

    let admin_token = args
        .windows(2)
        .find(|w| w[0] == "--token")
        .map(|w| w[1].clone())
        .or_else(|| std::env::var("AGENTDB_ADMIN_TOKEN").ok());

    // Read the seed file.
    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Failed to read {file_path}: {e}");
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "SEED_FILE_READ".into(),
                    message: msg,
                    span: None,
                    hint: Some(format!("Create a seed.json file or use --file <path>")),
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
    };

    // Parse as JSON object: { "EntityName": [ {row}, ... ], ... }
    let seed_data: BTreeMap<String, Vec<serde_json::Value>> = match serde_json::from_str(&content)
    {
        Ok(d) => d,
        Err(e) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "SEED_INVALID_JSON".into(),
                    message: format!("Invalid JSON in {file_path}: {e}"),
                    span: None,
                    hint: Some(
                        "Expected format: { \"Entity\": [ {row}, ... ], ... }".to_string(),
                    ),
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
    };

    let mut seeded: BTreeMap<String, usize> = BTreeMap::new();
    let mut total: usize = 0;
    let mut errors: Vec<String> = Vec::new();

    for (entity, rows) in &seed_data {
        let mut count = 0usize;
        for row in rows {
            let body = serde_json::to_string(row).unwrap_or_default();
            let path = format!("/api/entities/{entity}");
            match http_post("127.0.0.1", port, &path, &body, admin_token.as_deref()) {
                Ok((status, resp_body)) => {
                    if status >= 200 && status < 300 {
                        count += 1;
                        total += 1;
                    } else {
                        errors.push(format!(
                            "{entity}: HTTP {status} — {resp_body}",
                        ));
                    }
                }
                Err(e) => {
                    errors.push(format!("{entity}: connection error — {e}"));
                }
            }
        }
        seeded.insert(entity.clone(), count);
    }

    if !errors.is_empty() {
        let diags: Vec<Diagnostic> = errors
            .iter()
            .map(|e| Diagnostic {
                severity: Severity::Error,
                code: "SEED_INSERT_FAILED".into(),
                message: e.clone(),
                span: None,
                hint: None,
            })
            .collect();
        print_diagnostics(&diags, json_mode);
    }

    if json_mode {
        print_json(&SeedResult {
            code: if errors.is_empty() {
                "SEED_OK"
            } else {
                "SEED_PARTIAL"
            },
            seeded: seeded.clone(),
            total,
        });
    } else {
        let parts: Vec<String> = seeded
            .iter()
            .filter(|(_, &count)| count > 0)
            .map(|(entity, count)| format!("{count} {entity}"))
            .collect();
        if parts.is_empty() {
            eprintln!("No rows seeded.");
        } else {
            println!("Seeded {} ({total} total)", parts.join(", "));
        }
    }

    if errors.is_empty() {
        ExitCode::Ok
    } else if total > 0 {
        // Some rows succeeded, some failed.
        ExitCode::Error
    } else {
        ExitCode::Error
    }
}

/// Minimal HTTP POST using raw `TcpStream`.
///
/// Sends an HTTP/1.1 POST request and returns the status code and response
/// body. This avoids pulling in an HTTP client crate for a simple dev-time
/// seeding operation.
fn http_post(
    host: &str,
    port: u16,
    path: &str,
    body: &str,
    token: Option<&str>,
) -> Result<(u16, String), String> {
    let addr = format!("{host}:{port}");
    let mut stream =
        TcpStream::connect(&addr).map_err(|e| format!("connect to {addr}: {e}"))?;

    // Set a generous timeout so we don't hang forever on a stuck server.
    let timeout = std::time::Duration::from_secs(30);
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();

    let auth_header = match token {
        Some(t) => format!("Authorization: Bearer {t}\r\n"),
        None => String::new(),
    };

    let request = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         {auth_header}\
         \r\n\
         {body}",
        body.len(),
    );

    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write request: {e}"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|e| format!("read response: {e}"))?;

    // Parse status code from first line: "HTTP/1.1 201 Created\r\n..."
    let status = response
        .lines()
        .next()
        .and_then(|line| {
            let parts: Vec<&str> = line.splitn(3, ' ').collect();
            parts.get(1).and_then(|s| s.parse::<u16>().ok())
        })
        .unwrap_or(0);

    // Extract body: everything after the first blank line (\r\n\r\n).
    let resp_body = response
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or("")
        .to_string();

    Ok((status, resp_body))
}
