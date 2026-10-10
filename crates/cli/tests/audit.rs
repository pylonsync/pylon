//! `pylon audit` against a small site served from this test.
//!
//! A thread answers HTTP on a free port with fixed pages, robots.txt, and a
//! sitemap that names a production domain (rosas.example). The test runs the
//! real `pylon` binary with `--json --no-lighthouse` and checks the report and
//! the exit code. No Bun or Chrome is needed.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;

const HEAD: &str = r#"<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta property="og:title" content="Rosa's Bakery">
<meta property="og:description" content="Fresh bread in Austin.">
<meta property="og:image" content="https://rosas.example/opengraph-image">"#;

fn page(path: &str) -> Option<(u16, &'static str, String)> {
    let html = |title: &str, desc: &str, extra_head: &str, body: &str, canonical: &str| {
        format!(
            r#"<!doctype html><html lang="en"><head>{HEAD}
<title>{title}</title>{desc}
<link rel="canonical" href="https://rosas.example{canonical}">{extra_head}
</head><body>{body}</body></html>"#
        )
    };
    let desc = |d: &str| format!(r#"<meta name="description" content="{d}">"#);
    let ok = |s: String| Some((200, "text/html; charset=utf-8", s));
    match path {
        "/" => ok(html(
            "Rosa's Bakery | Fresh bread in Austin",
            &desc("Rosa's Bakery bakes sourdough, pastries and custom cakes every morning in East Austin."),
            r#"<script type="application/ld+json">{"@context":"https://schema.org","@type":"Bakery","name":"Rosa's Bakery"}</script>"#,
            r#"<h1>Rosa's Bakery</h1><a href="/menu">Menu</a> <a href="/about">About</a> <a href="/broken">Old page</a>"#,
            "/",
        )),
        "/menu" => ok(html(
            "Menu | Rosa's Bakery",
            "",
            "",
            r#"<h1>Menu</h1><img src="/bread.jpg"><a href="/">Home</a>"#,
            "/menu",
        )),
        "/about" => ok(html(
            "About Rosa's Bakery",
            &desc("Rosa started baking in 1998. The bakery moved to East Austin in 2015 and now bakes for 40 cafes."),
            "",
            "<h1>About</h1>",
            "/about",
        )),
        "/hidden" => ok(html(
            "Staff schedule | Rosa's Bakery",
            &desc("The weekly schedule for the bakery staff, with shifts for the front counter and the ovens."),
            r#"<meta name="robots" content="noindex">"#,
            "<h1>Schedule</h1>",
            "/hidden",
        )),
        "/opengraph-image" => Some((200, "image/png", "PNG".into())),
        "/robots.txt" => Some((
            200,
            "text/plain",
            "User-agent: *\nAllow: /\nSitemap: https://rosas.example/sitemap.xml\n".into(),
        )),
        "/sitemap.xml" => Some((
            200,
            "application/xml",
            r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
<url><loc>https://rosas.example/</loc></url>
<url><loc>https://rosas.example/menu</loc></url>
<url><loc>https://rosas.example/hidden</loc></url>
<url><loc>https://rosas.example/gone</loc></url>
</urlset>"#
                .into(),
        )),
        _ => None,
    }
}

fn serve(stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    // Drain the headers.
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
            break;
        }
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let (status, content_type, body) =
        page(path).unwrap_or((404, "text/html", "<h1>Not found</h1>".into()));
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = stream;
    let _ = stream.write_all(response.as_bytes());
}

/// Start the fixture site; returns its base URL.
fn start_site() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || serve(stream));
        }
    });
    format!("http://127.0.0.1:{port}")
}

fn audit(args: &[&str]) -> (i32, serde_json::Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_pylon"))
        .arg("audit")
        .args(args)
        .args(["--json", "--no-lighthouse"])
        // Run outside the repo so no pylon.manifest.json is picked up.
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let report = serde_json::from_str(&stdout).unwrap_or(serde_json::Value::Null);
    (out.status.code().unwrap_or(-1), report)
}

fn codes(issues: &serde_json::Value) -> Vec<&str> {
    issues
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect()
}

fn page_issues<'a>(report: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    let page = report["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["path"] == path)
        .unwrap_or_else(|| panic!("no page {path} in {report}"));
    &page["issues"]
}

#[test]
fn audit_reports_problems_by_page_and_fails() {
    let base = start_site();
    let (code, report) = audit(&[&base]);
    assert_eq!(code, 1, "errors give exit code 1: {report}");
    assert_eq!(report["page_source"], "sitemap");
    assert_eq!(report["pages_audited"], 4);

    // The home page is complete apart from its broken link. Its og:image
    // names the production domain and is fetched from the audit target.
    assert_eq!(codes(page_issues(&report, "/")), vec!["AUDIT_LINK_BROKEN"]);
    assert!(page_issues(&report, "/")[0]["message"]
        .as_str()
        .unwrap()
        .contains("/broken"));

    let menu = codes(page_issues(&report, "/menu"));
    assert!(menu.contains(&"AUDIT_DESCRIPTION_MISSING"), "{menu:?}");
    assert!(menu.contains(&"AUDIT_IMG_ALT_MISSING"), "{menu:?}");
    // Structured data is required on the home page only.
    assert!(!menu.contains(&"AUDIT_JSONLD_MISSING"), "{menu:?}");

    assert!(codes(page_issues(&report, "/hidden")).contains(&"AUDIT_NOINDEX_IN_SITEMAP"));
    assert_eq!(
        codes(page_issues(&report, "/gone")),
        vec!["AUDIT_HTTP_STATUS"]
    );

    // /about is linked and indexable but not in the sitemap. /broken is not
    // a page, so it is not listed as missing.
    let site = &report["site"];
    let missing: Vec<&str> = site
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "AUDIT_NOT_IN_SITEMAP")
        .map(|d| d["message"].as_str().unwrap())
        .collect();
    assert_eq!(missing.len(), 1, "{site}");
    assert!(missing[0].starts_with("/about "), "{missing:?}");

    assert_eq!(report["lighthouse"]["ran"], false);
    assert!(report["errors"].as_u64().unwrap() >= 5);
    // Every problem says how to fix it.
    for p in report["pages"].as_array().unwrap() {
        for d in p["issues"].as_array().unwrap() {
            assert!(!d["hint"].as_str().unwrap_or_default().is_empty(), "{d}");
        }
    }
}

#[test]
fn page_cap_and_human_output() {
    let base = start_site();
    let out = Command::new(env!("CARGO_BIN_EXE_pylon"))
        .args(["audit", &base, "--max-pages", "1", "--no-lighthouse"])
        .env("NO_COLOR", "1")
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("1 page(s) from the sitemap, 3 more not audited"),
        "{stdout}"
    );
    assert!(
        stdout.contains("error[AUDIT_LINK_BROKEN] The link to /broken returns 404."),
        "{stdout}"
    );
    assert!(stdout.contains("hint: Fix the link's href"), "{stdout}");
    assert!(
        stdout.contains("Lighthouse did not run: turned off"),
        "{stdout}"
    );
}

#[test]
fn nothing_listening_is_exit_69() {
    // Bind and drop to get a port nothing listens on.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (code, _) = audit(&[&format!("http://127.0.0.1:{port}")]);
    assert_eq!(code, 69);
}
