//! A TCP proxy in front of Postgres that reads the client's protocol
//! messages and records, per SQL text, how many times it was parsed
//! (a Parse message: one extra round trip with rust-postgres) and run
//! (a Bind message). Tests use it to pin how many statements a write costs.
//!
//! The proxy does not read TLS: connect through it with `sslmode=disable`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default, Clone)]
pub struct Counts {
    /// Parse messages per SQL text.
    pub parsed: HashMap<String, usize>,
    /// Bind messages (statement runs) per SQL text.
    pub run: HashMap<String, usize>,
}

impl Counts {
    /// Parses of statements whose SQL contains `needle`.
    pub fn parsed_matching(&self, needle: &str) -> usize {
        sum_matching(&self.parsed, needle)
    }

    /// Runs of statements whose SQL contains `needle`.
    pub fn run_matching(&self, needle: &str) -> usize {
        sum_matching(&self.run, needle)
    }
}

fn sum_matching(map: &HashMap<String, usize>, needle: &str) -> usize {
    map.iter()
        .filter(|(sql, _)| sql.contains(needle))
        .map(|(_, n)| n)
        .sum()
}

pub struct WireCounter {
    pub port: u16,
    counts: Arc<Mutex<Counts>>,
}

impl WireCounter {
    /// Listen on a free local port and forward each connection to
    /// `target` (`host:port`).
    pub fn start(target: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let counts = Arc::new(Mutex::new(Counts::default()));
        let shared = Arc::clone(&counts);
        std::thread::spawn(move || {
            for client in listener.incoming() {
                let Ok(client) = client else { return };
                let Ok(server) = TcpStream::connect(&target) else {
                    continue;
                };
                let _ = client.set_nodelay(true);
                let _ = server.set_nodelay(true);
                let (client_read, server_write) =
                    (client.try_clone().unwrap(), server.try_clone().unwrap());
                let counts = Arc::clone(&shared);
                std::thread::spawn(move || forward_client(client_read, server_write, counts));
                std::thread::spawn(move || copy(server, client));
            }
        });
        Self { port, counts }
    }

    /// Clear the counts.
    pub fn reset(&self) {
        *self.counts.lock().unwrap() = Counts::default();
    }

    /// The counts since the last reset.
    pub fn counts(&self) -> Counts {
        self.counts.lock().unwrap().clone()
    }
}

fn copy(mut from: TcpStream, mut to: TcpStream) {
    let mut buf = [0u8; 64 * 1024];
    while let Ok(n) = from.read(&mut buf) {
        if n == 0 || to.write_all(&buf[..n]).is_err() {
            break;
        }
    }
    let _ = to.shutdown(std::net::Shutdown::Write);
}

/// Forward client bytes to the server, reading the frontend messages.
fn forward_client(mut from: TcpStream, mut to: TcpStream, counts: Arc<Mutex<Counts>>) {
    let mut pending: Vec<u8> = Vec::new();
    let mut startup = true;
    // Statement name -> SQL, for this connection.
    let mut statements: HashMap<String, String> = HashMap::new();
    let mut buf = [0u8; 64 * 1024];
    while let Ok(n) = from.read(&mut buf) {
        if n == 0 || to.write_all(&buf[..n]).is_err() {
            break;
        }
        pending.extend_from_slice(&buf[..n]);
        loop {
            if startup {
                // Untyped: length, then a code. SSLRequest and
                // GSSENCRequest come before the startup message.
                if pending.len() < 8 {
                    break;
                }
                let len = u32::from_be_bytes(pending[0..4].try_into().unwrap()) as usize;
                if pending.len() < len {
                    break;
                }
                let code = u32::from_be_bytes(pending[4..8].try_into().unwrap());
                pending.drain(..len);
                if code != 80_877_103 && code != 80_877_104 {
                    startup = false;
                }
                continue;
            }
            if pending.len() < 5 {
                break;
            }
            let len = u32::from_be_bytes(pending[1..5].try_into().unwrap()) as usize;
            if pending.len() < 1 + len {
                break;
            }
            let kind = pending[0];
            let body = pending[5..1 + len].to_vec();
            pending.drain(..1 + len);
            match kind {
                b'P' => {
                    let (name, rest) = cstr(&body);
                    let (sql, _) = cstr(rest);
                    statements.insert(name.clone(), sql.clone());
                    *counts.lock().unwrap().parsed.entry(sql).or_default() += 1;
                }
                b'B' => {
                    let (_portal, rest) = cstr(&body);
                    let (name, _) = cstr(rest);
                    if let Some(sql) = statements.get(&name) {
                        *counts.lock().unwrap().run.entry(sql.clone()).or_default() += 1;
                    }
                }
                b'Q' => {
                    let (sql, _) = cstr(&body);
                    *counts.lock().unwrap().run.entry(sql).or_default() += 1;
                }
                _ => {}
            }
        }
    }
    let _ = to.shutdown(std::net::Shutdown::Write);
}

/// A NUL-terminated string and the bytes after it.
fn cstr(bytes: &[u8]) -> (String, &[u8]) {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let rest = bytes.get(end + 1..).unwrap_or(&[]);
    (String::from_utf8_lossy(&bytes[..end]).into_owned(), rest)
}
