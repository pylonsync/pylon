//! Uploaded files are served so they cannot run script on the app's origin.
//!
//! `/api/files/<id>` is on the app's own origin. An uploader chooses the
//! file's bytes and its declared type, so a file declared `text/html` (or
//! SVG, XML, JavaScript) would run its script with the app's cookies the
//! moment someone opened the link. These tests upload through the real
//! three-step flow (init, PUT, confirm) and check what the download
//! response tells the browser.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::*;
use pylon_runtime::Runtime;
use serde_json::{json, Value};

struct Reply {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

fn request(
    port: u16,
    method: &str,
    path: &str,
    body: &[u8],
    token: Option<&str>,
    ct: &str,
) -> Reply {
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\n{auth}Connection: close\r\n\r\n",
        body.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok();
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Reply {
        status,
        headers,
        body: body.to_string(),
    }
}

fn free_port() -> u16 {
    for _ in 0..200 {
        let base = 20_000 + rand::random::<u16>() % 8_000;
        if (0..4).all(|o| pylon_runtime::listen::port_is_free(base + o)) {
            return base;
        }
    }
    panic!("no free port block");
}

/// One server for the whole file: `PYLON_FILES_DIR` is process-wide.
fn server() -> (u16, String) {
    static SERVER: std::sync::OnceLock<(u16, String)> = std::sync::OnceLock::new();
    SERVER
        .get_or_init(|| {
            let dir =
                std::env::temp_dir().join(format!("pylon-file-serving-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            // SAFETY: once per binary, before the server thread starts.
            unsafe {
                std::env::set_var("PYLON_DEV_MODE", "1");
                std::env::set_var("PYLON_FILES_DIR", &dir);
            }
            let manifest = AppManifest {
                manifest_version: MANIFEST_VERSION,
                name: "file-serving".into(),
                version: "0.1.0".into(),
                ..Default::default()
            };
            let port = free_port();
            let runtime = Arc::new(Runtime::in_memory(manifest).unwrap());
            std::thread::spawn(move || {
                let _ = pylon_runtime::server::start(runtime, port);
            });
            let ready = (0..600).any(|_| {
                if TcpStream::connect(("127.0.0.1", port)).is_ok()
                    && request(port, "GET", "/health", b"", None, "text/plain").status == 200
                {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(50));
                false
            });
            assert!(ready, "server never answered /health");
            let r = request(
                port,
                "POST",
                "/api/auth/session",
                br#"{"user_id":"uploader"}"#,
                None,
                "application/json",
            );
            assert!(r.status == 200 || r.status == 201, "session: {}", r.body);
            let token = serde_json::from_str::<Value>(&r.body).unwrap()["token"]
                .as_str()
                .unwrap()
                .to_string();
            (port, token)
        })
        .clone()
}

fn init(mime: &str, size: usize) -> Reply {
    let (port, token) = server();
    let body = json!({"filename": "f", "mimeType": mime, "size": size, "visibility": "public"});
    request(
        port,
        "POST",
        "/api/files/init",
        body.to_string().as_bytes(),
        Some(&token),
        "application/json",
    )
}

/// Upload `bytes` declared as `mime` and return the download response.
fn upload_and_fetch(mime: &str, bytes: &[u8]) -> Reply {
    let (port, token) = server();
    let r = init(mime, bytes.len());
    assert_eq!(r.status, 200, "init {mime}: {}", r.body);
    let init: Value = serde_json::from_str(&r.body).unwrap();
    let asset = init["assetId"].as_str().unwrap();
    let upload_path = init["uploadUrl"]
        .as_str()
        .unwrap()
        .trim_start_matches(&format!("http://127.0.0.1:{port}"))
        .to_string();
    let r = request(port, "PUT", &upload_path, bytes, Some(&token), mime);
    assert!(r.status < 300, "put: {} {}", r.status, r.body);
    let r = request(
        port,
        "POST",
        "/api/files/confirm",
        json!({"assetId": asset}).to_string().as_bytes(),
        Some(&token),
        "application/json",
    );
    assert_eq!(r.status, 200, "confirm: {}", r.body);
    // Fetched by someone else: the file is public.
    request(
        port,
        "GET",
        &format!("/api/files/{asset}"),
        b"",
        None,
        "text/plain",
    )
}

fn assert_contained(r: &Reply) {
    assert_eq!(
        r.headers.get("x-content-type-options").map(String::as_str),
        Some("nosniff"),
        "{:?}",
        r.headers
    );
    let csp = r
        .headers
        .get("content-security-policy")
        .cloned()
        .unwrap_or_default();
    assert!(csp.contains("sandbox"), "no sandbox CSP: {:?}", r.headers);
}

const SCRIPT: &[u8] = b"<html><body><script>fetch('/api/auth/session')</script></body></html>";

#[test]
fn an_uploaded_html_file_is_a_download_not_a_page() {
    let r = upload_and_fetch("text/html", SCRIPT);
    assert_eq!(r.status, 200);
    assert_ne!(
        r.headers.get("content-type").map(String::as_str),
        Some("text/html"),
        "served as a page on the app origin: {:?}",
        r.headers
    );
    let disposition = r
        .headers
        .get("content-disposition")
        .cloned()
        .unwrap_or_default();
    assert!(disposition.starts_with("attachment"), "{:?}", r.headers);
    assert_contained(&r);
}

#[test]
fn svg_xml_and_javascript_are_downloads_too() {
    for mime in [
        "image/svg+xml",
        "application/xhtml+xml",
        "text/xml",
        "application/javascript",
        "TEXT/HTML; charset=utf-8",
    ] {
        let r = upload_and_fetch(mime, SCRIPT);
        let disposition = r
            .headers
            .get("content-disposition")
            .cloned()
            .unwrap_or_default();
        assert!(
            disposition.starts_with("attachment"),
            "{mime}: {:?}",
            r.headers
        );
        assert_eq!(
            r.headers.get("content-type").map(String::as_str),
            Some("application/octet-stream"),
            "{mime}"
        );
        assert_contained(&r);
    }
}

#[test]
fn images_audio_video_pdf_and_text_still_display_inline() {
    for mime in [
        "image/png",
        "image/jpeg",
        "video/mp4",
        "audio/mpeg",
        "text/plain",
    ] {
        let r = upload_and_fetch(mime, b"bytes");
        assert_eq!(r.status, 200);
        assert_eq!(
            r.headers.get("content-type").map(String::as_str),
            Some(mime),
            "{mime}"
        );
        assert!(
            !r.headers.contains_key("content-disposition"),
            "{mime}: {:?}",
            r.headers
        );
        assert_contained(&r);
    }
    // PDF keeps working in the browser's viewer, so no sandbox, but nosniff.
    let r = upload_and_fetch("application/pdf", b"%PDF-1.4");
    assert_eq!(
        r.headers.get("content-type").map(String::as_str),
        Some("application/pdf")
    );
    assert_eq!(
        r.headers.get("x-content-type-options").map(String::as_str),
        Some("nosniff")
    );
}

/// The declared type became a response header as is, so a line break in it
/// added headers of the uploader's choosing (Set-Cookie) to every download.
#[test]
fn a_declared_type_that_is_not_a_media_type_is_refused() {
    for mime in [
        "text/plain\r\nSet-Cookie: pwned=1",
        "text/plain\nX-Injected: 1",
        "image/pngé",
        "not a type",
        "",
    ] {
        let r = init(mime, 5);
        assert_eq!(r.status, 400, "{mime:?}: {}", r.body);
        assert!(r.body.contains("INVALID_MIME_TYPE"), "{mime:?}: {}", r.body);
    }
}
