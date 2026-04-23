//! `pylon cache` -- start a standalone cache server.
//!
//! This runs the pylon cache as an independent HTTP service, enabling
//! horizontal scaling by deploying the cache on a separate machine.
//!
//! Optionally starts a RESP-compatible TCP server alongside (or instead of)
//! the HTTP server. The RESP server speaks the Redis wire protocol, so any
//! `redis-cli` or Redis client library can connect directly.
//!
//! # Usage
//!
//! ```text
//! pylon cache [--port 6380] [--resp-port 6379] [--resp-only] [--max-keys 100000] [--max-history 100]
//! ```

use crate::output::print_error;
use pylon_kernel::ExitCode;

const DEFAULT_PORT: u16 = 6380;
const DEFAULT_RESP_PORT: u16 = 6379;
const DEFAULT_MAX_KEYS: usize = 100_000;
const DEFAULT_MAX_HISTORY: usize = 100;

pub fn run(args: &[String], _json_mode: bool) -> ExitCode {
    let port = parse_flag_u16(args, "--port").unwrap_or(DEFAULT_PORT);
    let max_keys = parse_flag_usize(args, "--max-keys").unwrap_or(DEFAULT_MAX_KEYS);
    let max_history = parse_flag_usize(args, "--max-history").unwrap_or(DEFAULT_MAX_HISTORY);
    let resp_only = args.iter().any(|a| a == "--resp-only");

    // --resp-port enables the RESP server. If --resp-only is set but
    // --resp-port is not explicitly provided, use the default Redis port.
    let resp_port: Option<u16> = if resp_only {
        Some(parse_flag_u16(args, "--resp-port").unwrap_or(DEFAULT_RESP_PORT))
    } else {
        parse_flag_u16(args, "--resp-port")
    };

    if args.iter().any(|a| a == "--help") {
        print_usage();
        return ExitCode::Ok;
    }

    eprintln!("Starting standalone cache server...");
    eprintln!("  port:        {port}");
    eprintln!("  max-keys:    {max_keys}");
    eprintln!("  max-history: {max_history}");
    if let Some(rp) = resp_port {
        eprintln!("  resp-port:   {rp}");
        eprintln!("  resp-only:   {resp_only}");
    }
    eprintln!();

    if !resp_only {
        eprintln!("  HTTP:  http://localhost:{port}/cache");
    }
    if let Some(rp) = resp_port {
        eprintln!("  RESP:  redis-cli -p {rp}");
    }
    eprintln!();

    match pylon_runtime::cache_server::start_cache_server_with_options(
        port,
        max_keys,
        max_history,
        resp_port,
        resp_only,
    ) {
        Ok(()) => ExitCode::Ok,
        Err(e) => {
            print_error(&e);
            ExitCode::Error
        }
    }
}

fn print_usage() {
    println!("pylon cache -- run a standalone cache server");
    println!();
    println!("Usage:");
    println!("  pylon cache [options]");
    println!();
    println!("Options:");
    println!("  --port <port>          HTTP listen port (default: {DEFAULT_PORT})");
    println!("  --resp-port <port>     RESP (Redis protocol) port (default: {DEFAULT_RESP_PORT})");
    println!("  --resp-only            Only start the RESP server, no HTTP");
    println!("  --max-keys <n>         Maximum cached keys (default: {DEFAULT_MAX_KEYS})");
    println!(
        "  --max-history <n>      Max pub/sub history per channel (default: {DEFAULT_MAX_HISTORY})"
    );
    println!("  --help                 Show this message");
    println!();
    println!("HTTP endpoints:");
    println!("  POST /cache            Execute a cache command");
    println!("  GET  /cache/:key       Get a key");
    println!("  DELETE /cache/:key     Delete a key");
    println!("  POST /pubsub/publish   Publish a message");
    println!("  GET  /pubsub/channels  List channels");
    println!("  GET  /pubsub/history/* Channel history");
    println!("  GET  /health           Health check");
    println!();
    println!("RESP server (when --resp-port is set):");
    println!("  Compatible with redis-cli and any Redis client library.");
    println!("  Supports: GET, SET, DEL, EXISTS, INCR, DECR, EXPIRE, TTL,");
    println!("            LPUSH, RPUSH, LPOP, RPOP, LRANGE, SADD, SREM,");
    println!("            SMEMBERS, HSET, HGET, HGETALL, ZADD, ZRANGE,");
    println!("            KEYS, DBSIZE, FLUSHALL, INFO, PING, and more.");
}

/// Parse a `--flag <value>` pair as u16.
fn parse_flag_u16(args: &[String], flag: &str) -> Option<u16> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
}

/// Parse a `--flag <value>` pair as usize.
fn parse_flag_usize(args: &[String], flag: &str) -> Option<usize> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
}
