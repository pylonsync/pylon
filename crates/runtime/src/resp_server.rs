//! RESP-compatible TCP server for the pylon cache.
//!
//! Speaks the Redis wire protocol (RESP2), so any `redis-cli` or Redis client
//! library can talk directly to the pylon cache without HTTP overhead.
//!
//! # Supported commands
//!
//! Strings: GET, SET, DEL, EXISTS, INCR, DECR, INCRBY, SETNX, GETSET, MGET, MSET
//! TTL:     EXPIRE, PERSIST, TTL
//! Lists:   LPUSH, RPUSH, LPOP, RPOP, LRANGE, LLEN
//! Sets:    SADD, SREM, SMEMBERS, SISMEMBER, SCARD, SINTER, SUNION
//! Hashes:  HSET, HGET, HDEL, HGETALL, HEXISTS, HLEN, HKEYS, HINCRBY
//! Sorted:  ZADD, ZREM, ZSCORE, ZRANK, ZRANGE, ZCARD
//! Keys:    KEYS, TYPE, DBSIZE, FLUSHALL, FLUSHDB
//! Conn:    PING, ECHO, QUIT, COMMAND, INFO

use std::io::{BufReader, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use pylon_plugin::builtin::cache::CachePlugin;

use crate::resp::{parse_resp, RespValue};

/// Default bind address — loopback only.
///
/// Historic behavior was dual-stack `[::]:port`, which bound to every
/// interface (LAN + the public internet on a misconfigured Fly machine)
/// with no authentication. That let anyone with network reach run
/// `FLUSHALL`, dump every key with `KEYS *`, etc. Loopback is the only
/// safe default; operators who want network reach must pass
/// `--bind 0.0.0.0`, `--bind-all`, or a specific bind address AND set
/// `--requirepass` / `PYLON_RESP_REQUIREPASS`.
pub const DEFAULT_RESP_BIND: &str = "127.0.0.1";

/// Start a RESP-compatible server (Redis protocol).
///
/// This blocks the calling thread. Each client connection is handled in
/// its own thread with a synchronous read loop.
///
/// # Authentication
///
/// When `requirepass` is `Some`, clients must issue `AUTH <password>`
/// before any command other than `PING`, `AUTH`, or `QUIT`. The password
/// comparison is constant-time (`pylon_auth::constant_time_eq`).
///
/// When `requirepass` is `None`, the server runs with no auth. A
/// stderr warning is logged if the bind address is non-loopback.
///
/// # Bind address
///
/// `bind_addr` accepts a host (e.g. `"127.0.0.1"`, `"0.0.0.0"`, `"::"`,
/// `"192.168.1.10"`). The special value `"0.0.0.0"` (or any non-loopback
/// IPv4) is also re-bound as dual-stack v6+v4 via [`crate::bind_dual_stack_tcp`]
/// when the operator passes the `--bind-all` shorthand from the CLI
/// (which sets `bind_addr = Some("0.0.0.0")` and `dual_stack = true`).
/// Pass `None` for the default loopback-only bind.
pub fn start_resp_server(
    cache: Arc<CachePlugin>,
    port: u16,
    bind_addr: Option<String>,
    requirepass: Option<String>,
) {
    let bind = bind_addr
        .clone()
        .unwrap_or_else(|| DEFAULT_RESP_BIND.to_string());

    let listener = match open_listener(&bind, port) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("[resp] Failed to bind RESP server on {bind}:{port}: {e}");
            return;
        }
    };

    let is_loopback = is_loopback_addr(&bind);
    if !is_loopback && requirepass.is_none() {
        // Loud stderr warning so an operator who turned on `--bind-all`
        // (or `--bind 0.0.0.0`) without a password sees this before
        // the next prod incident. tracing::warn! routes to stderr in
        // the default subscriber config.
        eprintln!(
            "[resp] WARNING: binding non-loopback address {bind} without --requirepass; \
             anyone with network reach can run FLUSHALL etc. \
             Set PYLON_RESP_REQUIREPASS=<password> or limit network reach."
        );
        tracing::warn!(
            "[resp] WARNING: binding non-loopback address {bind} without --requirepass; \
             anyone with network reach can run FLUSHALL etc. \
             Set PYLON_RESP_REQUIREPASS=<password> or limit network reach."
        );
    }

    tracing::warn!("[resp] RESP server listening on resp://{bind}:{port}");
    tracing::warn!("[resp] Compatible with redis-cli: redis-cli -h {bind} -p {port}");

    let requirepass = requirepass.map(Arc::new);

    loop {
        // Panic-proof accept: libstd's accept asserts (panics) on a truncated
        // macOS dual-stack sockaddr. See crate::accept_tcp.
        let (stream, _peer_ip) = match crate::accept_tcp(&listener) {
            Ok(v) => v,
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
        };

        let cache = Arc::clone(&cache);
        let requirepass = requirepass.clone();
        thread::spawn(move || {
            handle_client(cache, stream, requirepass);
        });
    }
}

/// Open a TCP listener on `bind_addr:port`.
///
/// - `"0.0.0.0"` and `"::"` go through `bind_dual_stack_tcp` so v6
///   clients still reach us on Linux + macOS.
/// - Every other host is bound directly (loopback or an explicit
///   interface).
fn open_listener(bind_addr: &str, port: u16) -> std::io::Result<TcpListener> {
    if bind_addr == "0.0.0.0" || bind_addr == "::" {
        crate::bind_dual_stack_tcp(port)
    } else {
        TcpListener::bind((bind_addr, port))
    }
}

/// Is `addr` a loopback address (no network reach beyond the local host)?
///
/// Accepts string forms — `"127.0.0.1"`, `"::1"`, `"localhost"` — and
/// any other 127.0.0.0/8 IPv4 address that parses cleanly.
fn is_loopback_addr(addr: &str) -> bool {
    if addr.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = addr.parse::<Ipv4Addr>() {
        return ip.is_loopback();
    }
    if let Ok(ip) = addr.parse::<Ipv6Addr>() {
        return ip.is_loopback();
    }
    if let Ok(sock) = addr.parse::<SocketAddr>() {
        return sock.ip().is_loopback();
    }
    false
}

fn handle_client(cache: Arc<CachePlugin>, stream: TcpStream, requirepass: Option<Arc<String>>) {
    let write_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);
    let mut writer = write_stream;

    // If no password is required, the connection is pre-authed. Otherwise,
    // every command except PING / AUTH / QUIT is rejected with NOAUTH
    // until a successful AUTH.
    let mut authed = requirepass.is_none();

    loop {
        let value = match parse_resp(&mut reader) {
            Ok(v) => v,
            Err(_) => break, // Client disconnected or protocol error
        };

        // Commands arrive as arrays: ["SET", "key", "value"]
        let args = match value {
            RespValue::Array(Some(items)) => items,
            _ => {
                let _ = writer.write_all(&RespValue::err("Expected array command").serialize());
                continue;
            }
        };

        let cmd_parts: Vec<String> = args
            .iter()
            .filter_map(|v| match v {
                RespValue::BulkString(Some(s)) => Some(s.clone()),
                RespValue::SimpleString(s) => Some(s.clone()),
                _ => None,
            })
            .collect();

        if cmd_parts.is_empty() {
            let _ = writer.write_all(&RespValue::err("Empty command").serialize());
            continue;
        }

        let upper = cmd_parts[0].to_uppercase();

        // AUTH handling — happens before the authed gate so it works
        // pre-auth (the whole point) and also post-auth (Redis lets you
        // re-AUTH; we accept it idempotently).
        if upper == "AUTH" {
            let response = handle_auth(requirepass.as_deref(), &cmd_parts, &mut authed);
            let _ = writer.write_all(&response.serialize());
            let _ = writer.flush();
            continue;
        }

        // Gate: pre-auth connections can only PING / QUIT.
        if !authed && upper != "PING" && upper != "QUIT" {
            let _ = writer
                .write_all(&RespValue::Error("NOAUTH Authentication required.".into()).serialize());
            let _ = writer.flush();
            continue;
        }

        let response = execute_command(&cache, &cmd_parts);
        let _ = writer.write_all(&response.serialize());
        let _ = writer.flush();

        // QUIT: send OK then close.
        if upper == "QUIT" {
            break;
        }
    }
}

/// Handle the `AUTH` command.
///
/// Uses constant-time compare so an attacker can't time the byte
/// match. Wrong passwords keep `authed = false` and return a generic
/// error (no information leak about whether AUTH is even required).
fn handle_auth(requirepass: Option<&String>, args: &[String], authed: &mut bool) -> RespValue {
    let expected = match requirepass {
        Some(pw) => pw,
        None => {
            // Redis returns this exact error when AUTH is sent but the
            // server has no requirepass configured.
            return RespValue::Error(
                "ERR Client sent AUTH, but no password is set. Did you mean AUTH <username> <password>?".into(),
            );
        }
    };

    // Redis accepts `AUTH <password>` (RESP2) and `AUTH <username> <password>`
    // (Redis 6+ ACLs). We only support the password form — we don't model
    // users — and we accept either by taking the LAST arg as the password.
    let presented = match args.len() {
        2 => &args[1],
        3 => &args[2],
        _ => {
            return RespValue::err("wrong number of arguments for 'auth' command");
        }
    };

    if pylon_auth::constant_time_eq(expected.as_bytes(), presented.as_bytes()) {
        *authed = true;
        RespValue::ok()
    } else {
        // Don't flip `authed` — wrong password keeps the connection gated.
        RespValue::Error("WRONGPASS invalid username-password pair or user is disabled.".into())
    }
}

/// Execute a Redis command against the CachePlugin.
fn execute_command(cache: &CachePlugin, args: &[String]) -> RespValue {
    let cmd = args[0].to_uppercase();

    match cmd.as_str() {
        // -----------------------------------------------------------------
        // Connection
        // -----------------------------------------------------------------
        "PING" => {
            if args.len() > 1 {
                RespValue::bulk(&args[1])
            } else {
                RespValue::SimpleString("PONG".into())
            }
        }
        "ECHO" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'echo' command");
            }
            RespValue::bulk(&args[1])
        }
        "QUIT" => RespValue::ok(),
        "COMMAND" => RespValue::ok(), // redis-cli sends this on connect

        // -----------------------------------------------------------------
        // Strings
        // -----------------------------------------------------------------
        "SET" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'set' command");
            }

            // Parse optional flags: EX seconds, PX milliseconds, NX, XX.
            let mut ttl: Option<u64> = None;
            let mut nx = false;
            let mut xx = false;
            let mut i = 3;
            while i < args.len() {
                match args[i].to_uppercase().as_str() {
                    "EX" => {
                        i += 1;
                        ttl = args.get(i).and_then(|v| v.parse::<u64>().ok());
                    }
                    "PX" => {
                        i += 1;
                        ttl = args.get(i).and_then(|v| v.parse::<u64>().ok()).map(|ms| {
                            // Convert ms to seconds, rounding up so sub-second TTLs
                            // still expire rather than becoming zero (infinite).
                            if ms == 0 {
                                0
                            } else {
                                (ms + 999) / 1000
                            }
                        });
                    }
                    "NX" => nx = true,
                    "XX" => xx = true,
                    _ => {}
                }
                i += 1;
            }

            if nx {
                if cache.setnx(&args[1], &args[2], ttl) {
                    RespValue::ok()
                } else {
                    RespValue::null()
                }
            } else if xx {
                if cache.exists(&args[1]) {
                    cache.set(&args[1], &args[2], ttl);
                    RespValue::ok()
                } else {
                    RespValue::null()
                }
            } else {
                cache.set(&args[1], &args[2], ttl);
                RespValue::ok()
            }
        }
        "GET" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'get' command");
            }
            match cache.get(&args[1]) {
                Some(v) => RespValue::bulk(&v),
                None => RespValue::null(),
            }
        }
        "DEL" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'del' command");
            }
            let mut count = 0i64;
            for key in &args[1..] {
                if cache.del(key) {
                    count += 1;
                }
            }
            RespValue::int(count)
        }
        "EXISTS" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'exists' command");
            }
            let mut count = 0i64;
            for key in &args[1..] {
                if cache.exists(key) {
                    count += 1;
                }
            }
            RespValue::int(count)
        }
        "INCR" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'incr' command");
            }
            match cache.incr(&args[1]) {
                Ok(n) => RespValue::int(n),
                Err(e) => RespValue::err(&e),
            }
        }
        "DECR" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'decr' command");
            }
            match cache.decr(&args[1]) {
                Ok(n) => RespValue::int(n),
                Err(e) => RespValue::err(&e),
            }
        }
        "INCRBY" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'incrby' command");
            }
            let amount: i64 = match args[2].parse() {
                Ok(n) => n,
                Err(_) => return RespValue::err("value is not an integer or out of range"),
            };
            match cache.incrby(&args[1], amount) {
                Ok(n) => RespValue::int(n),
                Err(e) => RespValue::err(&e),
            }
        }
        "SETNX" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'setnx' command");
            }
            let set = cache.setnx(&args[1], &args[2], None);
            RespValue::int(if set { 1 } else { 0 })
        }
        "GETSET" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'getset' command");
            }
            match cache.getset(&args[1], &args[2]) {
                Some(v) => RespValue::bulk(&v),
                None => RespValue::null(),
            }
        }
        "MGET" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'mget' command");
            }
            let keys: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();
            let values = cache.mget(&keys);
            RespValue::array(
                values
                    .into_iter()
                    .map(|v| match v {
                        Some(s) => RespValue::bulk(&s),
                        None => RespValue::null(),
                    })
                    .collect(),
            )
        }
        "MSET" => {
            if args.len() < 3 || (args.len() - 1) % 2 != 0 {
                return RespValue::err("wrong number of arguments for 'mset' command");
            }
            let mut pairs = Vec::new();
            let mut i = 1;
            while i < args.len() - 1 {
                pairs.push((args[i].as_str(), args[i + 1].as_str()));
                i += 2;
            }
            cache.mset(&pairs);
            RespValue::ok()
        }

        // -----------------------------------------------------------------
        // TTL
        // -----------------------------------------------------------------
        "EXPIRE" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'expire' command");
            }
            let secs: u64 = match args[2].parse() {
                Ok(n) => n,
                Err(_) => return RespValue::err("value is not an integer or out of range"),
            };
            RespValue::int(if cache.expire(&args[1], secs) { 1 } else { 0 })
        }
        "PERSIST" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'persist' command");
            }
            RespValue::int(if cache.persist(&args[1]) { 1 } else { 0 })
        }
        "TTL" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'ttl' command");
            }
            RespValue::int(cache.ttl(&args[1]))
        }

        // -----------------------------------------------------------------
        // Lists
        // -----------------------------------------------------------------
        "LPUSH" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'lpush' command");
            }
            let mut len = 0;
            for val in &args[2..] {
                len = cache.lpush(&args[1], val);
            }
            RespValue::int(len as i64)
        }
        "RPUSH" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'rpush' command");
            }
            let mut len = 0;
            for val in &args[2..] {
                len = cache.rpush(&args[1], val);
            }
            RespValue::int(len as i64)
        }
        "LPOP" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'lpop' command");
            }
            match cache.lpop(&args[1]) {
                Some(v) => RespValue::bulk(&v),
                None => RespValue::null(),
            }
        }
        "RPOP" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'rpop' command");
            }
            match cache.rpop(&args[1]) {
                Some(v) => RespValue::bulk(&v),
                None => RespValue::null(),
            }
        }
        "LRANGE" => {
            if args.len() < 4 {
                return RespValue::err("wrong number of arguments for 'lrange' command");
            }
            let start: i64 = args[2].parse().unwrap_or(0);
            let stop: i64 = args[3].parse().unwrap_or(-1);
            let items = cache.lrange(&args[1], start, stop);
            RespValue::array(items.into_iter().map(|s| RespValue::bulk(&s)).collect())
        }
        "LLEN" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'llen' command");
            }
            RespValue::int(cache.llen(&args[1]) as i64)
        }

        // -----------------------------------------------------------------
        // Sets
        // -----------------------------------------------------------------
        "SADD" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'sadd' command");
            }
            let mut added = 0i64;
            for member in &args[2..] {
                if cache.sadd(&args[1], member) {
                    added += 1;
                }
            }
            RespValue::int(added)
        }
        "SREM" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'srem' command");
            }
            let mut removed = 0i64;
            for member in &args[2..] {
                if cache.srem(&args[1], member) {
                    removed += 1;
                }
            }
            RespValue::int(removed)
        }
        "SMEMBERS" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'smembers' command");
            }
            let members = cache.smembers(&args[1]);
            RespValue::array(members.into_iter().map(|s| RespValue::bulk(&s)).collect())
        }
        "SISMEMBER" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'sismember' command");
            }
            RespValue::int(if cache.sismember(&args[1], &args[2]) {
                1
            } else {
                0
            })
        }
        "SCARD" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'scard' command");
            }
            RespValue::int(cache.scard(&args[1]) as i64)
        }
        "SINTER" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'sinter' command");
            }
            let result = cache.sinter(&args[1], &args[2]);
            RespValue::array(result.into_iter().map(|s| RespValue::bulk(&s)).collect())
        }
        "SUNION" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'sunion' command");
            }
            let result = cache.sunion(&args[1], &args[2]);
            RespValue::array(result.into_iter().map(|s| RespValue::bulk(&s)).collect())
        }

        // -----------------------------------------------------------------
        // Hashes
        // -----------------------------------------------------------------
        "HSET" => {
            if args.len() < 4 || (args.len() - 2) % 2 != 0 {
                return RespValue::err("wrong number of arguments for 'hset' command");
            }
            let mut count = 0i64;
            let mut i = 2;
            while i < args.len() - 1 {
                cache.hset(&args[1], &args[i], &args[i + 1]);
                count += 1;
                i += 2;
            }
            RespValue::int(count)
        }
        "HGET" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'hget' command");
            }
            match cache.hget(&args[1], &args[2]) {
                Some(v) => RespValue::bulk(&v),
                None => RespValue::null(),
            }
        }
        "HDEL" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'hdel' command");
            }
            let mut count = 0i64;
            for field in &args[2..] {
                if cache.hdel(&args[1], field) {
                    count += 1;
                }
            }
            RespValue::int(count)
        }
        "HGETALL" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'hgetall' command");
            }
            let map = cache.hgetall(&args[1]);
            let mut items = Vec::with_capacity(map.len() * 2);
            for (k, v) in &map {
                items.push(RespValue::bulk(k));
                items.push(RespValue::bulk(v));
            }
            RespValue::array(items)
        }
        "HEXISTS" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'hexists' command");
            }
            RespValue::int(if cache.hexists(&args[1], &args[2]) {
                1
            } else {
                0
            })
        }
        "HLEN" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'hlen' command");
            }
            RespValue::int(cache.hlen(&args[1]) as i64)
        }
        "HKEYS" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'hkeys' command");
            }
            let keys = cache.hkeys(&args[1]);
            RespValue::array(keys.into_iter().map(|s| RespValue::bulk(&s)).collect())
        }
        "HINCRBY" => {
            if args.len() < 4 {
                return RespValue::err("wrong number of arguments for 'hincrby' command");
            }
            let amount: i64 = match args[3].parse() {
                Ok(n) => n,
                Err(_) => return RespValue::err("value is not an integer or out of range"),
            };
            match cache.hincrby(&args[1], &args[2], amount) {
                Ok(n) => RespValue::int(n),
                Err(e) => RespValue::err(&e),
            }
        }

        // -----------------------------------------------------------------
        // Sorted sets
        // -----------------------------------------------------------------
        "ZADD" => {
            if args.len() < 4 || (args.len() - 2) % 2 != 0 {
                return RespValue::err("wrong number of arguments for 'zadd' command");
            }
            let mut count = 0i64;
            let mut i = 2;
            while i < args.len() - 1 {
                let score: f64 = match args[i].parse() {
                    Ok(n) => n,
                    Err(_) => return RespValue::err("value is not a valid float"),
                };
                cache.zadd(&args[1], score, &args[i + 1]);
                count += 1;
                i += 2;
            }
            RespValue::int(count)
        }
        "ZREM" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'zrem' command");
            }
            let mut count = 0i64;
            for member in &args[2..] {
                if cache.zrem(&args[1], member) {
                    count += 1;
                }
            }
            RespValue::int(count)
        }
        "ZSCORE" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'zscore' command");
            }
            match cache.zscore(&args[1], &args[2]) {
                Some(score) => RespValue::bulk(&format!("{score}")),
                None => RespValue::null(),
            }
        }
        "ZRANK" => {
            if args.len() < 3 {
                return RespValue::err("wrong number of arguments for 'zrank' command");
            }
            match cache.zrank(&args[1], &args[2]) {
                Some(rank) => RespValue::int(rank as i64),
                None => RespValue::null(),
            }
        }
        "ZRANGE" => {
            if args.len() < 4 {
                return RespValue::err("wrong number of arguments for 'zrange' command");
            }
            let start: usize = args[2].parse().unwrap_or(0);
            let stop: usize = args[3].parse().unwrap_or(0);
            let withscores = args[4..]
                .iter()
                .any(|a| a.eq_ignore_ascii_case("WITHSCORES"));
            let items = cache.zrange(&args[1], start, stop);
            if withscores {
                let mut result = Vec::with_capacity(items.len() * 2);
                for (member, score) in items {
                    result.push(RespValue::bulk(&member));
                    result.push(RespValue::bulk(&format!("{score}")));
                }
                RespValue::array(result)
            } else {
                RespValue::array(
                    items
                        .into_iter()
                        .map(|(m, _)| RespValue::bulk(&m))
                        .collect(),
                )
            }
        }
        "ZCARD" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'zcard' command");
            }
            RespValue::int(cache.zcard(&args[1]) as i64)
        }

        // -----------------------------------------------------------------
        // Keys / Server
        // -----------------------------------------------------------------
        "KEYS" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'keys' command");
            }
            let keys = cache.keys(&args[1]);
            RespValue::array(keys.into_iter().map(|s| RespValue::bulk(&s)).collect())
        }
        "TYPE" => {
            if args.len() < 2 {
                return RespValue::err("wrong number of arguments for 'type' command");
            }
            match cache.key_type(&args[1]) {
                Some(t) => RespValue::SimpleString(t.to_string()),
                None => RespValue::SimpleString("none".to_string()),
            }
        }
        "DBSIZE" => RespValue::int(cache.dbsize() as i64),
        "FLUSHALL" | "FLUSHDB" => {
            cache.flushall();
            RespValue::ok()
        }
        "INFO" => {
            let stats = cache.info();
            let info = format!(
                "# Server\r\nredis_version:pylon-resp\r\n\r\n\
                 # Stats\r\nhits:{}\r\nmisses:{}\r\nsets:{}\r\ndeletes:{}\r\nevictions:{}\r\nexpired:{}\r\n\r\n\
                 # Keyspace\r\nkeys:{}\r\n",
                stats.hits,
                stats.misses,
                stats.sets,
                stats.deletes,
                stats.evictions,
                stats.expired,
                cache.dbsize()
            );
            RespValue::bulk(&info)
        }

        _ => RespValue::err(&format!("unknown command '{cmd}'")),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resp::RespValue;
    use std::io::{BufReader, Cursor};

    /// Build a RESP command array from string slices and return the wire bytes.
    fn build_command(parts: &[&str]) -> Vec<u8> {
        let val = RespValue::array(parts.iter().map(|s| RespValue::bulk(s)).collect());
        val.serialize()
    }

    /// Simulate a client session: send raw RESP bytes and collect the response.
    ///
    /// Uses an in-memory buffer pair instead of real TCP sockets.
    fn run_session(cache: &CachePlugin, commands: &[u8]) -> Vec<u8> {
        run_session_with_auth(cache, commands, None)
    }

    /// Same as `run_session` but with an optional `requirepass`. Mirrors the
    /// production `handle_client` flow (NOAUTH gate + AUTH bookkeeping).
    fn run_session_with_auth(
        cache: &CachePlugin,
        commands: &[u8],
        requirepass: Option<&str>,
    ) -> Vec<u8> {
        let mut input = BufReader::new(Cursor::new(commands.to_vec()));
        let mut output = Vec::new();
        let mut authed = requirepass.is_none();
        let requirepass_owned = requirepass.map(|s| s.to_string());

        loop {
            let value = match crate::resp::parse_resp(&mut input) {
                Ok(v) => v,
                Err(_) => break,
            };

            let args = match value {
                RespValue::Array(Some(items)) => items,
                _ => {
                    output.extend_from_slice(&RespValue::err("Expected array command").serialize());
                    continue;
                }
            };

            let cmd_parts: Vec<String> = args
                .iter()
                .filter_map(|v| match v {
                    RespValue::BulkString(Some(s)) => Some(s.clone()),
                    RespValue::SimpleString(s) => Some(s.clone()),
                    _ => None,
                })
                .collect();

            if cmd_parts.is_empty() {
                output.extend_from_slice(&RespValue::err("Empty command").serialize());
                continue;
            }

            let upper = cmd_parts[0].to_uppercase();

            if upper == "AUTH" {
                let response = handle_auth(requirepass_owned.as_ref(), &cmd_parts, &mut authed);
                output.extend_from_slice(&response.serialize());
                continue;
            }

            if !authed && upper != "PING" && upper != "QUIT" {
                output.extend_from_slice(
                    &RespValue::Error("NOAUTH Authentication required.".into()).serialize(),
                );
                continue;
            }

            let response = execute_command(cache, &cmd_parts);
            output.extend_from_slice(&response.serialize());

            if upper == "QUIT" {
                break;
            }
        }

        output
    }

    /// Parse the first RESP value from raw bytes.
    fn parse_response(data: &[u8]) -> RespValue {
        let mut reader = BufReader::new(data);
        crate::resp::parse_resp(&mut reader).expect("Failed to parse response")
    }

    /// Parse all RESP values from raw bytes.
    fn parse_all_responses(data: &[u8]) -> Vec<RespValue> {
        let mut reader = BufReader::new(data);
        let mut results = Vec::new();
        loop {
            match crate::resp::parse_resp(&mut reader) {
                Ok(v) => results.push(v),
                Err(_) => break,
            }
        }
        results
    }

    // -- Connection commands --

    #[test]
    fn ping_pong() {
        let cache = CachePlugin::new(100);
        let output = run_session(&cache, &build_command(&["PING"]));
        assert_eq!(
            parse_response(&output),
            RespValue::SimpleString("PONG".into())
        );
    }

    #[test]
    fn ping_with_message() {
        let cache = CachePlugin::new(100);
        let output = run_session(&cache, &build_command(&["PING", "hello"]));
        assert_eq!(parse_response(&output), RespValue::bulk("hello"));
    }

    #[test]
    fn echo() {
        let cache = CachePlugin::new(100);
        let output = run_session(&cache, &build_command(&["ECHO", "test"]));
        assert_eq!(parse_response(&output), RespValue::bulk("test"));
    }

    #[test]
    fn quit() {
        let cache = CachePlugin::new(100);
        let output = run_session(&cache, &build_command(&["QUIT"]));
        assert_eq!(parse_response(&output), RespValue::ok());
    }

    // -- String commands --

    #[test]
    fn set_and_get() {
        let cache = CachePlugin::new(100);
        let mut cmds = build_command(&["SET", "mykey", "myval"]);
        cmds.extend_from_slice(&build_command(&["GET", "mykey"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::ok());
        assert_eq!(responses[1], RespValue::bulk("myval"));
    }

    #[test]
    fn get_nonexistent() {
        let cache = CachePlugin::new(100);
        let output = run_session(&cache, &build_command(&["GET", "nope"]));
        assert_eq!(parse_response(&output), RespValue::null());
    }

    #[test]
    fn del_multiple() {
        let cache = CachePlugin::new(100);
        cache.set("a", "1", None);
        cache.set("b", "2", None);

        let output = run_session(&cache, &build_command(&["DEL", "a", "b", "c"]));
        assert_eq!(parse_response(&output), RespValue::int(2));
    }

    #[test]
    fn exists() {
        let cache = CachePlugin::new(100);
        cache.set("x", "1", None);

        let mut cmds = build_command(&["EXISTS", "x"]);
        cmds.extend_from_slice(&build_command(&["EXISTS", "y"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::int(1));
        assert_eq!(responses[1], RespValue::int(0));
    }

    #[test]
    fn incr_decr() {
        let cache = CachePlugin::new(100);
        let mut cmds = build_command(&["INCR", "counter"]);
        cmds.extend_from_slice(&build_command(&["INCR", "counter"]));
        cmds.extend_from_slice(&build_command(&["DECR", "counter"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::int(1));
        assert_eq!(responses[1], RespValue::int(2));
        assert_eq!(responses[2], RespValue::int(1));
    }

    #[test]
    fn incrby() {
        let cache = CachePlugin::new(100);
        let output = run_session(&cache, &build_command(&["INCRBY", "k", "10"]));
        assert_eq!(parse_response(&output), RespValue::int(10));
    }

    #[test]
    fn setnx() {
        let cache = CachePlugin::new(100);
        let mut cmds = build_command(&["SETNX", "k", "first"]);
        cmds.extend_from_slice(&build_command(&["SETNX", "k", "second"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::int(1));
        assert_eq!(responses[1], RespValue::int(0));
    }

    #[test]
    fn set_nx_flag() {
        let cache = CachePlugin::new(100);
        cache.set("k", "existing", None);
        let output = run_session(&cache, &build_command(&["SET", "k", "new", "NX"]));
        assert_eq!(parse_response(&output), RespValue::null());
        assert_eq!(cache.get("k").unwrap(), "existing");
    }

    #[test]
    fn set_xx_flag() {
        let cache = CachePlugin::new(100);
        // XX on non-existent key should return null.
        let output = run_session(&cache, &build_command(&["SET", "k", "v", "XX"]));
        assert_eq!(parse_response(&output), RespValue::null());
        assert!(cache.get("k").is_none());
    }

    #[test]
    fn getset() {
        let cache = CachePlugin::new(100);
        cache.set("k", "old", None);
        let output = run_session(&cache, &build_command(&["GETSET", "k", "new"]));
        assert_eq!(parse_response(&output), RespValue::bulk("old"));
        assert_eq!(cache.get("k").unwrap(), "new");
    }

    #[test]
    fn mget_mset() {
        let cache = CachePlugin::new(100);
        let mut cmds = build_command(&["MSET", "a", "1", "b", "2"]);
        cmds.extend_from_slice(&build_command(&["MGET", "a", "b", "c"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::ok());
        assert_eq!(
            responses[1],
            RespValue::array(vec![
                RespValue::bulk("1"),
                RespValue::bulk("2"),
                RespValue::null(),
            ])
        );
    }

    // -- TTL commands --

    #[test]
    fn ttl_no_expiry() {
        let cache = CachePlugin::new(100);
        cache.set("k", "v", None);
        let output = run_session(&cache, &build_command(&["TTL", "k"]));
        assert_eq!(parse_response(&output), RespValue::int(-1));
    }

    #[test]
    fn expire_and_persist() {
        let cache = CachePlugin::new(100);
        cache.set("k", "v", None);

        let mut cmds = build_command(&["EXPIRE", "k", "60"]);
        cmds.extend_from_slice(&build_command(&["PERSIST", "k"]));
        cmds.extend_from_slice(&build_command(&["TTL", "k"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::int(1)); // EXPIRE ok
        assert_eq!(responses[1], RespValue::int(1)); // PERSIST ok
        assert_eq!(responses[2], RespValue::int(-1)); // TTL = no expiry
    }

    // -- List commands --

    #[test]
    fn lpush_rpush_lrange() {
        let cache = CachePlugin::new(100);
        let mut cmds = build_command(&["RPUSH", "list", "a", "b"]);
        cmds.extend_from_slice(&build_command(&["LPUSH", "list", "z"]));
        cmds.extend_from_slice(&build_command(&["LRANGE", "list", "0", "-1"]));
        cmds.extend_from_slice(&build_command(&["LLEN", "list"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::int(2)); // RPUSH
        assert_eq!(responses[1], RespValue::int(3)); // LPUSH
                                                     // LRANGE
        let items = match &responses[2] {
            RespValue::Array(Some(v)) => v.clone(),
            other => panic!("Expected array, got {other:?}"),
        };
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], RespValue::bulk("z"));
        assert_eq!(responses[3], RespValue::int(3)); // LLEN
    }

    #[test]
    fn lpop_rpop() {
        let cache = CachePlugin::new(100);
        cache.rpush("list", "a");
        cache.rpush("list", "b");
        cache.rpush("list", "c");

        let mut cmds = build_command(&["LPOP", "list"]);
        cmds.extend_from_slice(&build_command(&["RPOP", "list"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::bulk("a"));
        assert_eq!(responses[1], RespValue::bulk("c"));
    }

    // -- Set commands --

    #[test]
    fn sadd_smembers_scard() {
        let cache = CachePlugin::new(100);
        let mut cmds = build_command(&["SADD", "s", "a", "b", "a"]);
        cmds.extend_from_slice(&build_command(&["SCARD", "s"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::int(2)); // only 2 new
        assert_eq!(responses[1], RespValue::int(2));
    }

    #[test]
    fn sismember() {
        let cache = CachePlugin::new(100);
        cache.sadd("s", "x");

        let mut cmds = build_command(&["SISMEMBER", "s", "x"]);
        cmds.extend_from_slice(&build_command(&["SISMEMBER", "s", "y"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::int(1));
        assert_eq!(responses[1], RespValue::int(0));
    }

    // -- Hash commands --

    #[test]
    fn hset_hget_hgetall() {
        let cache = CachePlugin::new(100);
        let mut cmds = build_command(&["HSET", "h", "f1", "v1", "f2", "v2"]);
        cmds.extend_from_slice(&build_command(&["HGET", "h", "f1"]));
        cmds.extend_from_slice(&build_command(&["HLEN", "h"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::int(2));
        assert_eq!(responses[1], RespValue::bulk("v1"));
        assert_eq!(responses[2], RespValue::int(2));
    }

    #[test]
    fn hdel_hexists() {
        let cache = CachePlugin::new(100);
        cache.hset("h", "f", "v");

        let mut cmds = build_command(&["HEXISTS", "h", "f"]);
        cmds.extend_from_slice(&build_command(&["HDEL", "h", "f"]));
        cmds.extend_from_slice(&build_command(&["HEXISTS", "h", "f"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::int(1));
        assert_eq!(responses[1], RespValue::int(1));
        assert_eq!(responses[2], RespValue::int(0));
    }

    #[test]
    fn hincrby() {
        let cache = CachePlugin::new(100);
        let output = run_session(&cache, &build_command(&["HINCRBY", "h", "f", "5"]));
        assert_eq!(parse_response(&output), RespValue::int(5));
    }

    // -- Sorted set commands --

    #[test]
    fn zadd_zscore_zrank() {
        let cache = CachePlugin::new(100);
        let mut cmds = build_command(&["ZADD", "z", "1.5", "a", "2.5", "b"]);
        cmds.extend_from_slice(&build_command(&["ZSCORE", "z", "a"]));
        cmds.extend_from_slice(&build_command(&["ZRANK", "z", "b"]));
        cmds.extend_from_slice(&build_command(&["ZCARD", "z"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::int(2));
        assert_eq!(responses[1], RespValue::bulk("1.5"));
        assert_eq!(responses[2], RespValue::int(1));
        assert_eq!(responses[3], RespValue::int(2));
    }

    // -- Server commands --

    #[test]
    fn dbsize_and_flushall() {
        let cache = CachePlugin::new(100);
        cache.set("a", "1", None);
        cache.set("b", "2", None);

        let mut cmds = build_command(&["DBSIZE"]);
        cmds.extend_from_slice(&build_command(&["FLUSHALL"]));
        cmds.extend_from_slice(&build_command(&["DBSIZE"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::int(2));
        assert_eq!(responses[1], RespValue::ok());
        assert_eq!(responses[2], RespValue::int(0));
    }

    #[test]
    fn keys_pattern() {
        let cache = CachePlugin::new(100);
        cache.set("user:1", "a", None);
        cache.set("user:2", "b", None);
        cache.set("session:1", "c", None);

        let output = run_session(&cache, &build_command(&["KEYS", "user:*"]));
        let resp = parse_response(&output);
        match resp {
            RespValue::Array(Some(items)) => assert_eq!(items.len(), 2),
            other => panic!("Expected array, got {other:?}"),
        }
    }

    #[test]
    fn type_command() {
        let cache = CachePlugin::new(100);
        cache.set("str", "v", None);

        let mut cmds = build_command(&["TYPE", "str"]);
        cmds.extend_from_slice(&build_command(&["TYPE", "nonexistent"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses[0], RespValue::SimpleString("string".into()));
        assert_eq!(responses[1], RespValue::SimpleString("none".into()));
    }

    #[test]
    fn info_command() {
        let cache = CachePlugin::new(100);
        let output = run_session(&cache, &build_command(&["INFO"]));
        match parse_response(&output) {
            RespValue::BulkString(Some(s)) => {
                assert!(s.contains("hits:"));
                assert!(s.contains("keys:"));
            }
            other => panic!("Expected bulk string, got {other:?}"),
        }
    }

    #[test]
    fn unknown_command() {
        let cache = CachePlugin::new(100);
        let output = run_session(&cache, &build_command(&["FOOBAR"]));
        match parse_response(&output) {
            RespValue::Error(msg) => assert!(msg.contains("unknown command")),
            other => panic!("Expected error, got {other:?}"),
        }
    }

    // -- Argument validation --

    #[test]
    fn set_wrong_args() {
        let cache = CachePlugin::new(100);
        let output = run_session(&cache, &build_command(&["SET", "key"]));
        match parse_response(&output) {
            RespValue::Error(msg) => assert!(msg.contains("wrong number")),
            other => panic!("Expected error, got {other:?}"),
        }
    }

    // -- Multi-command session --

    // -- Security: bind defaults + AUTH gate --

    #[test]
    fn default_bind_is_loopback() {
        // The exported default must be a loopback address; an operator
        // who calls `start_resp_server(..., None, None)` must not get a
        // 0.0.0.0 bind.
        assert!(
            is_loopback_addr(DEFAULT_RESP_BIND),
            "DEFAULT_RESP_BIND ({DEFAULT_RESP_BIND}) is not loopback"
        );
        assert!(is_loopback_addr("127.0.0.1"));
        assert!(is_loopback_addr("::1"));
        assert!(is_loopback_addr("localhost"));
        assert!(!is_loopback_addr("0.0.0.0"));
        assert!(!is_loopback_addr("::"));
        assert!(!is_loopback_addr("192.168.1.10"));
    }

    #[test]
    fn flushall_without_auth_returns_noauth() {
        let cache = CachePlugin::new(100);
        cache.set("a", "1", None);

        let output = run_session_with_auth(&cache, &build_command(&["FLUSHALL"]), Some("hunter2"));
        match parse_response(&output) {
            RespValue::Error(msg) => assert!(
                msg.starts_with("NOAUTH"),
                "expected NOAUTH error, got: {msg}"
            ),
            other => panic!("Expected NOAUTH error, got {other:?}"),
        }

        // Critically: FLUSHALL must NOT have actually run.
        assert_eq!(cache.dbsize(), 1, "FLUSHALL ran despite NOAUTH gate");
    }

    #[test]
    fn ping_allowed_pre_auth() {
        // PING must still work pre-auth so health probes / connection
        // tests don't need to AUTH first.
        let cache = CachePlugin::new(100);
        let output = run_session_with_auth(&cache, &build_command(&["PING"]), Some("hunter2"));
        assert_eq!(
            parse_response(&output),
            RespValue::SimpleString("PONG".into())
        );
    }

    #[test]
    fn auth_correct_password_permits_subsequent_commands() {
        let cache = CachePlugin::new(100);
        let mut cmds = build_command(&["AUTH", "hunter2"]);
        cmds.extend_from_slice(&build_command(&["SET", "k", "v"]));
        cmds.extend_from_slice(&build_command(&["GET", "k"]));

        let responses = parse_all_responses(&run_session_with_auth(&cache, &cmds, Some("hunter2")));
        assert_eq!(responses[0], RespValue::ok()); // AUTH ok
        assert_eq!(responses[1], RespValue::ok()); // SET ok
        assert_eq!(responses[2], RespValue::bulk("v")); // GET v
    }

    #[test]
    fn auth_wrong_password_keeps_gate_closed() {
        let cache = CachePlugin::new(100);
        cache.set("a", "1", None);

        let mut cmds = build_command(&["AUTH", "wrong-password"]);
        cmds.extend_from_slice(&build_command(&["FLUSHALL"]));

        let responses = parse_all_responses(&run_session_with_auth(&cache, &cmds, Some("hunter2")));

        // AUTH returns WRONGPASS error.
        match &responses[0] {
            RespValue::Error(msg) => assert!(
                msg.starts_with("WRONGPASS"),
                "expected WRONGPASS, got: {msg}"
            ),
            other => panic!("Expected WRONGPASS error, got {other:?}"),
        }

        // FLUSHALL still gated.
        match &responses[1] {
            RespValue::Error(msg) => assert!(
                msg.starts_with("NOAUTH"),
                "expected NOAUTH after wrong AUTH, got: {msg}"
            ),
            other => panic!("Expected NOAUTH error, got {other:?}"),
        }

        // Cache untouched.
        assert_eq!(cache.dbsize(), 1, "FLUSHALL ran despite WRONGPASS + NOAUTH");
    }

    #[test]
    fn auth_uses_constant_time_compare() {
        // We can't measure timing in a unit test, but we can verify the
        // call routes through `pylon_auth::constant_time_eq` by checking
        // that same-length and different-length wrong passwords both
        // return WRONGPASS (and not some byte-length short-circuit).
        let cache = CachePlugin::new(100);

        // Same length as expected ("hunter2" is 7 bytes).
        let same_len = run_session_with_auth(
            &cache,
            &build_command(&["AUTH", "hunter1"]),
            Some("hunter2"),
        );
        // Different length.
        let diff_len =
            run_session_with_auth(&cache, &build_command(&["AUTH", "x"]), Some("hunter2"));

        for output in [&same_len, &diff_len] {
            match parse_response(output) {
                RespValue::Error(msg) => assert!(
                    msg.starts_with("WRONGPASS"),
                    "expected WRONGPASS, got: {msg}"
                ),
                other => panic!("Expected WRONGPASS, got {other:?}"),
            }
        }

        // And same password authenticates.
        let mut authed = false;
        let args = vec!["AUTH".to_string(), "hunter2".to_string()];
        let response = handle_auth(Some(&"hunter2".to_string()), &args, &mut authed);
        assert_eq!(response, RespValue::ok());
        assert!(authed);
    }

    #[test]
    fn auth_with_no_requirepass_returns_err() {
        // Sending AUTH against a server with no password configured
        // returns the standard Redis error.
        let cache = CachePlugin::new(100);
        let output = run_session_with_auth(&cache, &build_command(&["AUTH", "anything"]), None);
        match parse_response(&output) {
            RespValue::Error(msg) => assert!(
                msg.starts_with("ERR Client sent AUTH"),
                "expected 'no password is set' error, got: {msg}"
            ),
            other => panic!("Expected error, got {other:?}"),
        }
    }

    #[test]
    fn full_session() {
        let cache = CachePlugin::new(100);
        let mut cmds = Vec::new();
        cmds.extend_from_slice(&build_command(&["PING"]));
        cmds.extend_from_slice(&build_command(&["SET", "greeting", "hello"]));
        cmds.extend_from_slice(&build_command(&["GET", "greeting"]));
        cmds.extend_from_slice(&build_command(&["QUIT"]));

        let responses = parse_all_responses(&run_session(&cache, &cmds));
        assert_eq!(responses.len(), 4);
        assert_eq!(responses[0], RespValue::SimpleString("PONG".into()));
        assert_eq!(responses[1], RespValue::ok());
        assert_eq!(responses[2], RespValue::bulk("hello"));
        assert_eq!(responses[3], RespValue::ok()); // QUIT
    }
}
