//! A fake Steam Web API for `ISteamUserAuth/AuthenticateUserTicket/v1`.
//!
//! A ticket here is the hex of `steamid:appid:identity[:banned]`. The fake
//! answers as Steam does: an error when the request's app id or identity
//! differs from the ticket's, else `result: "OK"` with the SteamID. The
//! ticket `dead` (hex of "down") makes it close the connection without an
//! answer, as an unreachable Steam would.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

pub const DOWN_TICKET: &str = "646f776e";

/// The hex ticket for a player.
pub fn ticket(steam_id: &str, app_id: &str, identity: &str, banned: bool) -> String {
    let raw = if banned {
        format!("{steam_id}:{app_id}:{identity}:banned")
    } else {
        format!("{steam_id}:{app_id}:{identity}")
    };
    raw.bytes().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<String> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

fn query_value(query: &str, key: &str) -> String {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
        .unwrap_or("")
        .replace("%20", " ")
        .replace("%2D", "-")
}

fn answer(query: &str) -> Option<String> {
    let ticket = query_value(query, "ticket");
    if ticket == DOWN_TICKET {
        return None;
    }
    let error = |code: i64, desc: &str| {
        Some(
            serde_json::json!({"response":{"error":{"errorcode":code,"errordesc":desc}}})
                .to_string(),
        )
    };
    if query_value(query, "key") != "test-steam-key" {
        return error(3, "Invalid key");
    }
    let Some(raw) = unhex(&ticket) else {
        return error(101, "Invalid ticket");
    };
    let parts: Vec<&str> = raw.split(':').collect();
    if parts.len() < 3 {
        return error(101, "Invalid ticket");
    }
    if parts[1] != query_value(query, "appid") {
        return error(102, "Ticket for other app");
    }
    if parts[2] != query_value(query, "identity") {
        return error(101, "Invalid ticket");
    }
    let banned = parts.get(3) == Some(&"banned");
    Some(
        serde_json::json!({"response":{"params":{
            "result":"OK","steamid":parts[0],"ownersteamid":parts[0],
            "vacbanned":banned,"publisherbanned":false
        }}})
        .to_string(),
    )
}

/// Start the fake on a free port and return its base URL.
pub fn start() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                // Drain the headers.
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                        break;
                    }
                }
                let target = line.split_whitespace().nth(1).unwrap_or("");
                let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
                let Some(body) = answer(query) else {
                    return; // drop the connection
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    base
}
