//! `pylon add <integration>` — wire a Stack0 suite app into this project.
//!
//! Only Analytics needs this. Feedback is one script tag with nothing to get
//! wrong, and the docs say so; Analytics needs a relay function and a
//! first-party tracker route, and both fail silently when they are wrong.
//!
//! Why a generator instead of a template default:
//!
//!   * The site key does not exist at `pylon init` time. A scaffold could
//!     only ship a placeholder, which either no-ops (dead code everyone
//!     deletes) or posts to a site that is not theirs.
//!   * Pylon is open source. Every new project phoning home to a commercial
//!     analytics product by default is the wrong shape.
//!
//! The generated relay is the part worth automating. Written by hand it goes
//! wrong in ways that produce no error at all: a route handler instead of a
//! function (never matched, because `/api/fn/*` is Pylon's own namespace),
//! `args` instead of `rawBody` (drops fields the arg list does not declare),
//! or no relay at all (the upstream CORS allowlist refuses the browser's
//! preflight, which looks exactly like a quiet week).

use std::path::{Path, PathBuf};

use pylon_kernel::ExitCode;

use crate::output;

const ANALYTICS_ORIGIN: &str = "https://analytics.stack0.dev";
const DOCS: &str = "https://www.stack0.dev/docs/sdk/analytics";

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('-') && *a != "add")
        .collect();

    match positional.first().copied() {
        Some("analytics") => add_analytics(args, json_mode),
        Some("feedback") => {
            // Nothing to generate, so say what to do rather than writing a
            // file for the sake of having written one.
            print_feedback_help(args);
            ExitCode::Ok
        }
        Some(other) => {
            output::print_error(&format!("Unknown integration \"{other}\"."));
            eprintln!("  Available: analytics, feedback");
            ExitCode::Usage
        }
        None => {
            eprintln!("Usage: pylon add <analytics|feedback> [--site-key KEY]");
            eprintln!();
            eprintln!("  analytics  Relay function + first-party tracker route + layout tag");
            eprintln!("  feedback   Prints the one script tag it needs");
            ExitCode::Usage
        }
    }
}

fn flag_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().map(String::as_str);
        }
        if let Some(rest) = a.strip_prefix(&format!("{name}=")) {
            return Some(rest);
        }
    }
    None
}

fn print_feedback_help(args: &[String]) {
    let slug = flag_value(args, "--project").unwrap_or("YOUR_PORTAL_SLUG");
    println!("Stack0 Feedback is one tag. Put it in app/layout.tsx:");
    println!();
    println!("  <script");
    println!("    src=\"https://feedback.stack0.app/widget.js\"");
    println!("    data-project=\"{slug}\"");
    println!("    defer");
    println!("  />");
    println!();
    println!("Pylon renders it server-side, so there is no client code to write.");
    println!("Reading the board needs no key: https://www.stack0.dev/docs/sdk/feedback");
}

fn add_analytics(args: &[String], json_mode: bool) -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if !cwd.join("app.ts").is_file() {
        output::print_error("No app.ts here — run this from a Pylon project root.");
        return ExitCode::Usage;
    }

    let site_key = match flag_value(args, "--site-key") {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => {
            output::print_error("A site key is required.");
            eprintln!("  Add a site at {ANALYTICS_ORIGIN} — the key is public and goes in the tag.");
            eprintln!();
            eprintln!("  pylon add analytics --site-key <KEY>");
            return ExitCode::Usage;
        }
    };

    let relay = cwd.join("functions/ingestEvent.ts");
    let tracker = cwd.join("app/rt/track.js/route.ts");
    let mut wrote: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    for (path, body) in [
        (&relay, relay_source()),
        (&tracker, tracker_source()),
    ] {
        let rel = path.strip_prefix(&cwd).unwrap_or(path).display().to_string();
        if path.exists() {
            // Never clobber. Someone who already has a relay has probably
            // changed it, and a generator that eats edits is worse than one
            // that does nothing.
            skipped.push(rel);
            continue;
        }
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                output::print_error(&format!("Could not create {}: {e}", parent.display()));
                return ExitCode::Error;
            }
        }
        if let Err(e) = std::fs::write(path, body) {
            output::print_error(&format!("Could not write {rel}: {e}"));
            return ExitCode::Error;
        }
        wrote.push(rel);
    }

    let tag = format!("<script defer src=\"/rt/track.js\" data-site=\"{site_key}\" />");
    let layout_done = try_insert_tag(&cwd.join("app/layout.tsx"), &tag);

    if json_mode {
        let out = serde_json::json!({
            "integration": "analytics",
            "wrote": wrote,
            "skipped": skipped,
            "layoutPatched": layout_done,
            "tag": tag,
        });
        println!("{}", serde_json::to_string(&out).unwrap_or_default());
        return ExitCode::Ok;
    }

    for f in &wrote {
        println!("  wrote    {f}");
    }
    for f in &skipped {
        println!("  kept     {f} (already exists)");
    }
    match layout_done {
        true => println!("  patched  app/layout.tsx"),
        false => {
            println!();
            println!("Add the tag to app/layout.tsx yourself:");
            println!();
            println!("  {tag}");
        }
    }
    println!();
    println!("The relay is named ingestEvent, so track.js finds it without data-endpoint.");
    println!("Send a pageview, then confirm the beacon returns 200 in the network tab —");
    println!("a refused preflight looks exactly like no traffic.");
    println!("{DOCS}");
    ExitCode::Ok
}

/// Put the tag before `</body>` in the layout.
///
/// Returns false and leaves the file alone when the shape is not the obvious
/// one: printing a tag for someone to paste beats rewriting a file we did not
/// parse.
fn try_insert_tag(layout: &Path, tag: &str) -> bool {
    let Ok(src) = std::fs::read_to_string(layout) else {
        return false;
    };
    if src.contains("/rt/track.js") {
        return true; // already wired
    }
    let Some(idx) = src.rfind("</body>") else {
        return false;
    };
    let line_start = src[..idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let indent: String = src[line_start..idx]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let insert = format!("{indent}  {tag}\n{indent}");
    let patched = format!("{}{}{}", &src[..line_start], insert, &src[idx..]);
    std::fs::write(layout, patched).is_ok()
}

fn relay_source() -> String {
    format!(
        r#"// Stack0 Analytics beacon relay. Generated by `pylon add analytics`.
//
// This has to exist. Beacons sent straight to {origin} are refused by its
// CORS preflight, and a refused preflight is indistinguishable from nobody
// visiting the site. Relaying also survives ad blockers and preserves the
// visitor's location.
//
// It is an ACTION because only actions get `ctx.request`, and it forwards
// `rawBody` rather than `args` because the beacon carries fields this arg
// list does not declare.
import {{ action }} from "@pylonsync/functions";

const UPSTREAM = "{origin}/api/fn/ingestEvent";

// Forward whichever set your edge writes. Without these every visitor lands
// at the centre of their country instead of in their city.
const FORWARD = [
  "user-agent",
  "x-forwarded-for",
  // Cloudflare
  "cf-ipcountry",
  "cf-region-code",
  "cf-ipcity",
  "cf-iplatitude",
  "cf-iplongitude",
  // Vercel
  "x-vercel-ip-country",
  "x-vercel-ip-country-region",
  "x-vercel-ip-city",
  "x-vercel-ip-latitude",
  "x-vercel-ip-longitude",
];

export default action({{
  auth: "public",
  args: {{}},
  async handler(ctx) {{
    const req = ctx.request;
    if (!req) return {{}};

    const headers: Record<string, string> = {{ "content-type": "application/json" }};
    for (const name of FORWARD) {{
      const value = req.headers[name];
      if (value) headers[name] = value;
    }}

    try {{
      const res = await fetch(UPSTREAM, {{
        method: "POST",
        headers,
        body: req.rawBody,
      }});
      // Verbatim: track.js reads visitorId off this response.
      return await res.json();
    }} catch {{
      // Analytics is never worth failing a page over.
      return {{}};
    }}
  }},
}});
"#,
        origin = ANALYTICS_ORIGIN
    )
}

fn tracker_source() -> String {
    format!(
        r#"// Serves Stack0 Analytics' tracker from this origin. Generated by
// `pylon add analytics`.
//
// First-party so an ad blocker that knows the Analytics hostname does not
// take it out. Cached in process: the file changes rarely and this saves a
// round trip per cold start.
import type {{ RawRouteHandler }} from "@pylonsync/react";

const UPSTREAM = "{origin}/track.js";
// Empty rather than null on purpose. With `string | null`, the await on the
// right of a `??` re-widens the binding and `body` types as string | null,
// which RawResponse rejects. An empty string is never a valid tracker, so
// falsiness is the same test with none of that.
let cached = "";

export const GET: RawRouteHandler = async () => {{
  if (!cached) {{
    cached = await fetch(UPSTREAM).then((r) => r.text());
  }}
  return {{
    body: cached,
    contentType: "application/javascript; charset=utf-8",
    headers: {{ "cache-control": "public, max-age=3600" }},
  }};
}};
"#,
        origin = ANALYTICS_ORIGIN
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_relay_is_an_action_that_forwards_raw_body() {
        let src = relay_source();
        // These three are the whole point of generating the file: each one
        // is silent when wrong.
        assert!(src.contains("import { action }"), "must be an action");
        assert!(src.contains("req.rawBody"), "must forward rawBody");
        assert!(!src.contains("args.body"), "must not forward args");
        assert!(src.contains("auth: \"public\""));
    }

    #[test]
    fn the_relay_never_fails_the_page() {
        assert!(relay_source().contains("} catch {"));
    }

    #[test]
    fn the_tracker_route_is_a_raw_route_handler() {
        let src = tracker_source();
        assert!(src.contains("RawRouteHandler"));
        assert!(src.contains("application/javascript"));
    }

    #[test]
    fn the_tracker_cache_is_never_nullable() {
        // RawResponse.body is string | undefined. A `string | null` cache
        // does not narrow across the await — verified against tsc, not
        // assumed — so the cache starts empty instead of null.
        let src = tracker_source();
        assert!(
            !src.contains("let cached: string | null"),
            "a nullable cache does not typecheck as a RawResponse body"
        );
        assert!(src.contains("let cached = \"\";"));
        assert!(src.contains("body: cached"));
    }

    #[test]
    fn flags_parse_in_both_forms() {
        let split = vec!["add".into(), "analytics".into(), "--site-key".into(), "abc".into()];
        assert_eq!(flag_value(&split, "--site-key"), Some("abc"));
        let joined = vec!["add".into(), "analytics".into(), "--site-key=xyz".into()];
        assert_eq!(flag_value(&joined, "--site-key"), Some("xyz"));
        let absent: Vec<String> = vec!["add".into(), "analytics".into()];
        assert_eq!(flag_value(&absent, "--site-key"), None);
    }

    fn layout(body: &str) -> String {
        format!("export default function Layout() {{\n  return (\n    <html>\n      <body>\n{body}      </body>\n    </html>\n  );\n}}\n")
    }

    #[test]
    fn the_tag_lands_before_the_closing_body() {
        let dir = std::env::temp_dir().join(format!("pylon-add-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("layout.tsx");
        std::fs::write(&f, layout("        {children}\n")).unwrap();

        assert!(try_insert_tag(&f, "<script defer src=\"/rt/track.js\" data-site=\"k\" />"));
        let out = std::fs::read_to_string(&f).unwrap();
        let tag_at = out.find("/rt/track.js").unwrap();
        let body_at = out.find("</body>").unwrap();
        assert!(tag_at < body_at, "tag must precede </body>:\n{out}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_second_run_does_not_add_the_tag_twice() {
        let dir = std::env::temp_dir().join(format!("pylon-add2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("layout.tsx");
        std::fs::write(&f, layout("        {children}\n")).unwrap();
        let tag = "<script defer src=\"/rt/track.js\" data-site=\"k\" />";

        try_insert_tag(&f, tag);
        try_insert_tag(&f, tag);
        let out = std::fs::read_to_string(&f).unwrap();
        assert_eq!(out.matches("/rt/track.js").count(), 1, "{out}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unrecognised_layout_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("pylon-add3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("layout.tsx");
        // No </body> — a fragment layout, or something we did not parse.
        std::fs::write(&f, "export default function L() { return <div/>; }\n").unwrap();
        let before = std::fs::read_to_string(&f).unwrap();

        assert!(!try_insert_tag(&f, "<script/>"));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), before);
        std::fs::remove_dir_all(&dir).ok();
    }
}
