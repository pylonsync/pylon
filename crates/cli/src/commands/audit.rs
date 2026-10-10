//! `pylon audit` — check a running app the way a search engine and
//! Lighthouse do, and say how to fix each problem.
//!
//! Steps:
//!
//!   1. Find the pages: the URLs in the sitemap (from robots.txt `Sitemap:`
//!      lines, else /sitemap.xml), else the static routes in
//!      pylon.manifest.json, else the links on the home page. `--max-pages`
//!      caps the count.
//!   2. Check robots.txt and every sitemap file.
//!   3. Fetch each page and run the HTML checks in `audit_page.rs`, then the
//!      checks across pages (duplicate titles and descriptions).
//!   4. Fetch every internal link once and report the broken ones, fetch each
//!      og:image, and list linked pages that the sitemap leaves out.
//!   5. Run Lighthouse (performance, accessibility) on the first pages when
//!      Chrome and Bun are present. See `audit_lighthouse.rs`.
//!
//! Errors make the exit code 1. Warnings do not.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use pylon_kernel::{Diagnostic, ExitCode, Severity};
use url::Url;

use super::audit_lighthouse::{self, LighthouseScores, LIGHTHOUSE_VERSION};
use super::audit_page::{self, diag, is_loopback_host, PageContext, PageFacts};
use super::audit_site::{self, SitemapKind};
use crate::output;

const DEFAULT_PORT: u16 = 4321;
const DEFAULT_MAX_PAGES: usize = 50;
const DEFAULT_LIGHTHOUSE_PAGES: usize = 3;
/// Distinct internal links fetched beyond the audited pages.
const MAX_LINKS_CHECKED: usize = 300;
/// Sitemap files read, counting the files a sitemap index points to.
const MAX_SITEMAP_FILES: usize = 20;
const MAX_REDIRECTS: usize = 5;
const PAGE_MAX_BYTES: u64 = 5_000_000;
/// The sitemap protocol allows 50 MB per file, uncompressed.
const SITEMAP_MAX_BYTES: u64 = 50_000_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

pub const USAGE: &str = "\
Usage:
  pylon audit [url] [--max-pages <n>] [--lighthouse-pages <n>] [--no-lighthouse] [--json]

Checks a running app for search and page-quality problems: titles, meta
descriptions, headings, canonical links, lang, viewport, Open Graph tags,
image alt text, JSON-LD, broken internal links, robots.txt, and sitemap.xml.
With Chrome or Chromium present, it also runs Lighthouse for performance and
accessibility. Each problem comes with a one-line fix.

Arguments:
  url                      The app to audit (default: http://localhost:<port>)

Options:
  -p, --port <n>           Port of the local app (default: PYLON_PORT, else 4321)
  --max-pages <n>          Most pages to audit (default: 50)
  --lighthouse-pages <n>   Pages to run Lighthouse on, home first (default: 3)
  --no-lighthouse          Do not run Lighthouse
  --json                   Machine-readable report
  -h, --help               Show this help

Environment:
  CHROME_PATH              Chrome or Chromium for Lighthouse. Without it, the
                           audit looks for chromium or google-chrome on PATH.

Exit status: 0 when there are no errors (warnings are allowed), 1 when there
are errors, 69 when nothing answers at the URL.";

#[derive(Debug)]
struct Options {
    target: Url,
    max_pages: usize,
    lighthouse_pages: usize,
}

fn parse_count(args: &[String], flag: &str, default: usize) -> Result<usize, String> {
    match super::args::flag_value(args, flag) {
        None => Ok(default),
        Some(v) => v
            .parse::<usize>()
            .map_err(|_| format!("{flag} takes a whole number, not \"{v}\"")),
    }
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let given = super::args::flag_value(args, "--url").or_else(|| {
        super::args::collect_positional(args, "audit")
            .first()
            .map(|s| s.to_string())
    });
    let target = match given {
        Some(raw) => parse_target(&raw)?,
        None => {
            // The port `pylon dev` would use: .env files count, as they do for dev.
            let _ = super::dev::load_env_files();
            let port = super::args::parse_port(args, DEFAULT_PORT);
            Url::parse(&format!("http://localhost:{port}/")).map_err(|e| e.to_string())?
        }
    };
    let max_pages = parse_count(args, "--max-pages", DEFAULT_MAX_PAGES)?;
    if max_pages == 0 {
        return Err("--max-pages must be 1 or more".into());
    }
    let lighthouse_pages = if args.iter().any(|a| a == "--no-lighthouse") {
        0
    } else {
        parse_count(args, "--lighthouse-pages", DEFAULT_LIGHTHOUSE_PAGES)?
    };
    Ok(Options {
        target,
        max_pages,
        lighthouse_pages,
    })
}

/// Accept `https://x.com`, `x.com`, and `localhost:3000`.
fn parse_target(raw: &str) -> Result<Url, String> {
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else {
        let host = raw.split(['/', ':']).next().unwrap_or_default();
        let scheme = if is_loopback_host(host) {
            "http"
        } else {
            "https"
        };
        format!("{scheme}://{raw}")
    };
    let url = Url::parse(&with_scheme).map_err(|e| format!("\"{raw}\" is not a URL: {e}"))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(format!("\"{raw}\" is not an http or https URL"));
    }
    if url.host_str().is_none() {
        return Err(format!("\"{raw}\" has no host"));
    }
    Ok(url)
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
pub struct AuditReport {
    pub target: String,
    /// Where the page list came from: "sitemap", "routes", or "links".
    pub page_source: &'static str,
    pub pages_found: usize,
    pub pages_audited: usize,
    pub site: Vec<Diagnostic>,
    pub pages: Vec<PageReport>,
    pub lighthouse: LighthouseRun,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct PageReport {
    /// Path and query, as audited.
    pub path: String,
    /// The URL after redirects.
    pub url: String,
    /// None when the request failed.
    pub status: Option<u16>,
    pub lighthouse: Option<LighthouseScores>,
    pub issues: Vec<Diagnostic>,
}

#[derive(Debug, serde::Serialize)]
pub struct LighthouseRun {
    pub ran: bool,
    pub version: &'static str,
    pub chrome: Option<String>,
    pub pages: usize,
    /// Why Lighthouse did not run, or stopped.
    pub skipped: Option<String>,
}

// ---------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Fetched {
    /// The URL after redirects.
    url: Url,
    status: u16,
    content_type: Option<String>,
    x_robots_tag: Option<String>,
    body: String,
    redirects: usize,
}

impl Fetched {
    fn is_html(&self) -> bool {
        self.content_type
            .as_deref()
            .is_some_and(|c| c.to_ascii_lowercase().contains("text/html"))
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(REQUEST_TIMEOUT)
        // Redirects are followed by hand, to report them.
        .redirects(0)
        .user_agent(&format!("pylon-audit/{}", env!("CARGO_PKG_VERSION")))
        .build()
}

/// GET `url`, following up to five redirects. `Err` only when no HTTP
/// response came back; a 404 is an `Ok` with that status.
fn fetch(agent: &ureq::Agent, url: &Url, max_bytes: u64) -> Result<Fetched, String> {
    let mut current = url.clone();
    let mut redirects = 0;
    loop {
        let resp = match agent.get(current.as_str()).call() {
            Ok(r) | Err(ureq::Error::Status(_, r)) => r,
            Err(e) => return Err(e.to_string()),
        };
        let status = resp.status();
        if (300..400).contains(&status) {
            if let Some(location) = resp.header("location") {
                if redirects >= MAX_REDIRECTS {
                    return Err(format!("more than {MAX_REDIRECTS} redirects"));
                }
                current = current
                    .join(location)
                    .map_err(|e| format!("bad redirect to \"{location}\": {e}"))?;
                redirects += 1;
                continue;
            }
        }
        let content_type = resp.header("content-type").map(str::to_string);
        let x_robots_tag = resp.header("x-robots-tag").map(str::to_string);
        let mut bytes = Vec::new();
        resp.into_reader()
            .take(max_bytes)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("reading the response failed: {e}"))?;
        return Ok(Fetched {
            url: current,
            status,
            content_type,
            x_robots_tag,
            body: String::from_utf8_lossy(&bytes).into_owned(),
            redirects,
        });
    }
}

/// `/path?query`, the key pages are tracked by.
fn path_and_query(u: &Url) -> String {
    match u.query() {
        Some(q) => format!("{}?{q}", u.path()),
        None => u.path().to_string(),
    }
}

/// `host:port`, with the scheme's default port filled in.
fn authority(u: &Url) -> Option<String> {
    Some(format!(
        "{}:{}",
        u.host_str()?.to_ascii_lowercase(),
        u.port_or_known_default()?
    ))
}

/// Same-site check key for paths: `/about/` and `/about` are one page.
fn page_key(path: &str) -> String {
    audit_page::normalize_path(path).to_string()
}

// ---------------------------------------------------------------------------
// The audit
// ---------------------------------------------------------------------------

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let opts = match parse_options(args) {
        Ok(o) => o,
        Err(e) => {
            output::print_error(&e);
            eprintln!();
            eprintln!("{USAGE}");
            return ExitCode::Usage;
        }
    };
    let routes = manifest_page_routes(Path::new("pylon.manifest.json"));
    let progress = |msg: &str| {
        if !json_mode {
            eprintln!("→ {msg}");
        }
    };
    let report = match audit(&opts, &routes, &progress) {
        Ok(r) => r,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Unavailable;
        }
    };
    if json_mode {
        output::print_json(&report);
    } else {
        print_report(&report);
    }
    if report.errors > 0 {
        ExitCode::Error
    } else {
        ExitCode::Ok
    }
}

/// Static page routes from the manifest: no `:param`, no catch-all, and no
/// data routes (sitemap, robots, og-image, route handlers).
fn manifest_page_routes(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(manifest) = serde_json::from_str::<pylon_kernel::AppManifest>(&text) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for r in &manifest.routes {
        let is_page = matches!(r.kind.as_deref(), None | Some("page"));
        if is_page && !r.path.contains([':', '*', '[']) && !out.contains(&r.path) {
            out.push(r.path.clone());
        }
    }
    out
}

/// One fetched page and what was found on it.
struct PageState {
    path: String,
    fetched: Result<Fetched, String>,
    facts: Option<PageFacts>,
    issues: Vec<Diagnostic>,
    lighthouse: Option<LighthouseScores>,
}

/// What a link target returned.
#[derive(Clone)]
struct LinkResult {
    /// None when no response came back.
    status: Option<u16>,
    /// A 200 HTML page without noindex.
    indexable_page: bool,
    error: Option<String>,
}

fn audit(
    opts: &Options,
    routes: &[String],
    progress: &dyn Fn(&str),
) -> Result<AuditReport, String> {
    let agent = agent();
    let target = &opts.target;
    let origin = Url::parse(&target.origin().ascii_serialization())
        .map_err(|e| format!("{target} has no origin: {e}"))?;
    let target_is_local = target.host_str().is_some_and(is_loopback_host);
    let home_path = path_and_query(target);
    let mut site: Vec<Diagnostic> = Vec::new();
    // Hosts that count as this site. The sitemap and canonical links name the
    // production domain, which differs from a local audit target.
    let mut site_hosts: BTreeSet<String> = authority(target).into_iter().collect();

    progress(&format!("Auditing {target}"));
    let home = fetch(&agent, target, PAGE_MAX_BYTES).map_err(|e| {
        format!(
            "Nothing answers at {target} ({e}). Start the app with `pylon dev`, or pass its URL: pylon audit https://example.com"
        )
    })?;

    // robots.txt
    let robots_url = origin.join("/robots.txt").map_err(|e| e.to_string())?;
    let robots = match fetch(&agent, &robots_url, PAGE_MAX_BYTES) {
        Ok(f) if f.status == 200 && f.is_html() => {
            site.push(diag(
                Severity::Error,
                "AUDIT_ROBOTS_NOT_TEXT",
                "/robots.txt returns an HTML page, not a robots.txt file.".into(),
                "Add app/robots.ts that returns the crawl rules.",
            ));
            None
        }
        Ok(f) if f.status == 200 => Some(audit_site::parse_robots(&f.body)),
        Ok(f) if f.status == 404 || f.status == 410 => {
            site.push(diag(
                Severity::Warning,
                "AUDIT_ROBOTS_MISSING",
                format!("/robots.txt returns {}.", f.status),
                "Add app/robots.ts: allow \"/\" and list the sitemap URL.",
            ));
            None
        }
        Ok(f) => {
            site.push(diag(
                Severity::Error,
                "AUDIT_ROBOTS_ERROR",
                format!("/robots.txt returns {}. Google stops crawling the site while robots.txt fails.", f.status),
                "Make /robots.txt return 200 (app/robots.ts), or 404 if there are no rules.",
            ));
            None
        }
        Err(e) => {
            site.push(diag(
                Severity::Error,
                "AUDIT_ROBOTS_ERROR",
                format!("/robots.txt did not answer: {e}."),
                "Make /robots.txt return 200 (app/robots.ts), or 404 if there are no rules.",
            ));
            None
        }
    };

    // Sitemaps: the ones robots.txt names, else /sitemap.xml.
    let mut queue: Vec<Url> = Vec::new();
    if let Some(r) = &robots {
        for line in &r.sitemaps {
            match Url::parse(line) {
                Ok(u) => {
                    site_hosts.extend(authority(&u));
                    queue.push(rebase(&u, &origin, &site_hosts));
                }
                Err(_) => site.push(diag(
                    Severity::Error,
                    "AUDIT_ROBOTS_SITEMAP_RELATIVE",
                    format!("The robots.txt Sitemap line \"{line}\" is not an absolute URL."),
                    "Write the full sitemap URL, with https:// and the domain.",
                )),
            }
        }
    }
    if queue.is_empty() {
        queue.push(origin.join("/sitemap.xml").map_err(|e| e.to_string())?);
    }
    let mut sitemap_paths: Vec<String> = Vec::new();
    let mut have_sitemap = false;
    let mut seen_sitemaps: BTreeSet<String> = BTreeSet::new();
    while let Some(url) = queue.pop() {
        if !seen_sitemaps.insert(url.to_string()) {
            continue;
        }
        if seen_sitemaps.len() > MAX_SITEMAP_FILES {
            site.push(diag(
                Severity::Info,
                "AUDIT_SITEMAP_NOT_ALL_READ",
                format!("Read the first {MAX_SITEMAP_FILES} sitemap files only."),
                "Nothing to fix.",
            ));
            break;
        }
        let path = path_and_query(&url);
        let fetched = match fetch(&agent, &url, SITEMAP_MAX_BYTES) {
            Ok(f) => f,
            Err(e) => {
                site.push(diag(
                    Severity::Error,
                    "AUDIT_SITEMAP_ERROR",
                    format!("{path} did not answer: {e}."),
                    "Make the sitemap URL return 200 with the sitemap XML.",
                ));
                continue;
            }
        };
        if fetched.status == 404 || fetched.status == 410 {
            site.push(diag(
                Severity::Error,
                "AUDIT_SITEMAP_MISSING",
                format!("{path} returns {}.", fetched.status),
                "Add app/sitemap.ts that returns every public page.",
            ));
            continue;
        }
        if fetched.status != 200 {
            site.push(diag(
                Severity::Error,
                "AUDIT_SITEMAP_ERROR",
                format!("{path} returns {}.", fetched.status),
                "Make the sitemap URL return 200 with the sitemap XML.",
            ));
            continue;
        }
        let doc = match audit_site::parse_sitemap(&fetched.body) {
            Ok(d) => d,
            Err(e) => {
                site.push(diag(
                    Severity::Error,
                    "AUDIT_SITEMAP_INVALID",
                    format!("{path} is not a valid sitemap: {e}."),
                    "Generate it with app/sitemap.ts (default export returns [{ url }, ...]).",
                ));
                continue;
            }
        };
        site.extend(audit_site::sitemap_diagnostics(
            &path,
            &doc,
            target_is_local,
        ));
        for loc in &doc.locs {
            let Ok(u) = Url::parse(loc) else { continue };
            site_hosts.extend(authority(&u));
            match doc.kind {
                SitemapKind::Index => queue.push(rebase(&u, &origin, &site_hosts)),
                SitemapKind::UrlSet => {
                    let p = path_and_query(&u);
                    if !sitemap_paths.contains(&p) {
                        sitemap_paths.push(p);
                    }
                }
            }
        }
        if doc.kind == SitemapKind::UrlSet {
            have_sitemap = true;
        }
    }
    if let Some(r) = &robots {
        site.extend(audit_site::robots_diagnostics(r, &sitemap_paths));
    }
    let sitemap_keys: BTreeSet<String> = sitemap_paths.iter().map(|p| page_key(p)).collect();

    // The page list.
    let (page_source, mut paths): (&'static str, Vec<String>) = if !sitemap_paths.is_empty() {
        ("sitemap", sitemap_paths.clone())
    } else if !routes.is_empty() {
        ("routes", routes.to_vec())
    } else {
        let facts = if home.is_html() {
            audit_page::parse_page(&home.body)
        } else {
            PageFacts::default()
        };
        let mut found = Vec::new();
        for u in audit_page::resolve_links(&facts, &home.url) {
            if authority(&u).is_some_and(|a| site_hosts.contains(&a)) {
                let p = path_and_query(&u);
                if !found.contains(&p) {
                    found.push(p);
                }
            }
        }
        ("links", found)
    };
    paths.retain(|p| page_key(p) != page_key(&home_path));
    paths.insert(0, home_path.clone());
    let pages_found = paths.len();
    paths.truncate(opts.max_pages);
    progress(&format!(
        "Checking {} page(s) from the {}",
        paths.len(),
        match page_source {
            "sitemap" => "sitemap",
            "routes" => "app routes",
            _ => "home page links",
        }
    ));

    // Fetch and check each page.
    let mut pages: Vec<PageState> = Vec::new();
    for path in &paths {
        let fetched = if *path == home_path {
            Ok(home.clone())
        } else {
            let url = origin.join(path).map_err(|e| e.to_string())?;
            fetch(&agent, &url, PAGE_MAX_BYTES)
        };
        let in_sitemap = sitemap_keys.contains(&page_key(path));
        let mut issues = Vec::new();
        let mut facts = None;
        match &fetched {
            Err(e) => issues.push(diag(
                Severity::Error,
                "AUDIT_FETCH_FAILED",
                format!("The request failed: {e}."),
                "Open the page in a browser and fix the server error.",
            )),
            Ok(f) => {
                if f.redirects > 0 && in_sitemap {
                    issues.push(diag(
                        Severity::Warning,
                        "AUDIT_SITEMAP_REDIRECT",
                        format!(
                            "The sitemap lists this URL, but it redirects to {}.",
                            path_and_query(&f.url)
                        ),
                        "List the final URL in the sitemap.",
                    ));
                }
                if f.status >= 400 {
                    let fix = if in_sitemap {
                        "Fix the page, or remove it from the sitemap."
                    } else {
                        "Fix the page so it renders, or remove the route."
                    };
                    issues.push(diag(
                        Severity::Error,
                        "AUDIT_HTTP_STATUS",
                        format!("The page returns {}.", f.status),
                        fix,
                    ));
                } else if f.is_html() {
                    let parsed = audit_page::parse_page(&f.body);
                    let ctx = PageContext {
                        url: &f.url,
                        is_home: *path == home_path,
                        in_sitemap,
                        x_robots_tag: f.x_robots_tag.as_deref(),
                        target_is_local,
                    };
                    issues.extend(audit_page::check_page(&parsed, &ctx));
                    for c in &parsed.canonicals {
                        if let Ok(u) = Url::parse(c) {
                            site_hosts.extend(authority(&u));
                        }
                    }
                    facts = Some(parsed);
                }
            }
        }
        pages.push(PageState {
            path: path.clone(),
            fetched,
            facts,
            issues,
            lighthouse: None,
        });
    }

    // Duplicate titles and descriptions, among indexable pages.
    let indexable: Vec<(String, &PageFacts)> = pages
        .iter()
        .filter_map(|p| {
            let f = p.fetched.as_ref().ok()?;
            let facts = p.facts.as_ref()?;
            (f.status == 200 && !audit_page::is_noindex(facts, f.x_robots_tag.as_deref()))
                .then(|| (p.path.clone(), facts))
        })
        .collect();
    let mut dupes = audit_page::check_duplicates(&indexable);
    for p in &mut pages {
        if let Some(d) = dupes.remove(&p.path) {
            p.issues.extend(d);
        }
    }

    // Internal links. Known results first: the pages fetched above.
    let mut link_results: HashMap<String, LinkResult> = HashMap::new();
    for p in &pages {
        link_results.insert(page_key(&p.path), link_result(&p.fetched));
    }
    // Link target (path) → the pages that link to it.
    let mut linked_from: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for p in &pages {
        let (Some(facts), Ok(f)) = (&p.facts, &p.fetched) else {
            continue;
        };
        for u in audit_page::resolve_links(facts, &f.url) {
            if !authority(&u).is_some_and(|a| site_hosts.contains(&a)) {
                continue;
            }
            let target_path = path_and_query(&u);
            if target_path.starts_with("/_pylon/") {
                continue;
            }
            linked_from
                .entry(target_path)
                .or_default()
                .insert(p.path.clone());
        }
    }
    // Manifest routes are checked too, for the missing-from-sitemap list.
    let mut to_check: Vec<String> = linked_from.keys().cloned().collect();
    if have_sitemap {
        to_check.extend(routes.iter().cloned());
    }
    let unchecked: Vec<String> = {
        let mut seen = BTreeSet::new();
        to_check
            .into_iter()
            .filter(|p| !link_results.contains_key(&page_key(p)) && seen.insert(page_key(p)))
            .collect()
    };
    if !unchecked.is_empty() {
        progress(&format!(
            "Checking {} linked page(s)",
            unchecked.len().min(MAX_LINKS_CHECKED)
        ));
    }
    if unchecked.len() > MAX_LINKS_CHECKED {
        site.push(diag(
            Severity::Info,
            "AUDIT_LINKS_NOT_ALL_CHECKED",
            format!(
                "Checked the first {MAX_LINKS_CHECKED} of {} linked pages.",
                unchecked.len()
            ),
            "Nothing to fix.",
        ));
    }
    for path in unchecked.iter().take(MAX_LINKS_CHECKED) {
        let result = match origin.join(path) {
            Ok(url) => link_result(&fetch(&agent, &url, PAGE_MAX_BYTES)),
            Err(e) => LinkResult {
                status: None,
                indexable_page: false,
                error: Some(e.to_string()),
            },
        };
        link_results.insert(page_key(path), result);
    }
    for (target_path, from) in &linked_from {
        let Some(result) = link_results.get(&page_key(target_path)) else {
            continue;
        };
        let broken = match (result.status, &result.error) {
            (Some(s), _) if s >= 400 => Some(format!("returns {s}")),
            (None, Some(e)) => Some(format!("does not answer ({e})")),
            _ => None,
        };
        let Some(broken) = broken else { continue };
        for p in pages.iter_mut().filter(|p| from.contains(&p.path)) {
            p.issues.push(diag(
                Severity::Error,
                "AUDIT_LINK_BROKEN",
                format!("The link to {target_path} {broken}."),
                "Fix the link's href, or add the page it points to.",
            ));
        }
    }

    // og:image: the URL must return an image.
    let mut image_results: HashMap<String, Option<String>> = HashMap::new();
    for p in &mut pages {
        let Some(img) = p.facts.as_ref().and_then(|f| f.og_image.clone()) else {
            continue;
        };
        let Ok(u) = Url::parse(&img) else { continue };
        if u.scheme() != "http" && u.scheme() != "https" {
            continue;
        }
        let problem = image_results
            .entry(img.clone())
            .or_insert_with(|| check_image(&agent, &rebase(&u, &origin, &site_hosts)))
            .clone();
        if let Some(problem) = problem {
            p.issues.push(diag(
                Severity::Error,
                "AUDIT_OG_IMAGE_BROKEN",
                format!("og:image {img} {problem}."),
                "Make the image URL return the image: check the opengraph-image file and its render errors.",
            ));
        }
    }

    // Pages that are linked or routed, indexable, and not in the sitemap.
    if have_sitemap {
        let mut missing: BTreeSet<String> = BTreeSet::new();
        let candidates = linked_from.keys().chain(routes.iter()).chain(paths.iter());
        for path in candidates {
            if path.contains('?') || sitemap_keys.contains(&page_key(path)) {
                continue;
            }
            if link_results
                .get(&page_key(path))
                .is_some_and(|r| r.indexable_page)
            {
                missing.insert(page_key(path));
            }
        }
        for path in missing {
            site.push(diag(
                Severity::Warning,
                "AUDIT_NOT_IN_SITEMAP",
                format!("{path} is a public page, but the sitemap does not list it."),
                "Add it to app/sitemap.ts, or add noindex if search engines should skip it.",
            ));
        }
    }

    // Lighthouse.
    let lighthouse = run_lighthouse_pages(opts, &mut pages, progress);

    let pages: Vec<PageReport> = pages
        .into_iter()
        .map(|p| {
            let (url, status) = match &p.fetched {
                Ok(f) => (f.url.to_string(), Some(f.status)),
                Err(_) => (
                    origin
                        .join(&p.path)
                        .map(|u| u.to_string())
                        .unwrap_or_default(),
                    None,
                ),
            };
            PageReport {
                path: p.path,
                url,
                status,
                lighthouse: p.lighthouse,
                issues: p.issues,
            }
        })
        .collect();
    let all = site
        .iter()
        .chain(pages.iter().flat_map(|p| p.issues.iter()));
    let (errors, warnings) = all.fold((0, 0), |(e, w), d| match d.severity {
        Severity::Error => (e + 1, w),
        Severity::Warning => (e, w + 1),
        Severity::Info => (e, w),
    });
    Ok(AuditReport {
        target: target.to_string(),
        page_source,
        pages_found,
        pages_audited: pages.len(),
        site,
        pages,
        lighthouse,
        errors,
        warnings,
    })
}

/// A URL on another host of this site (the production domain named in the
/// sitemap or a canonical link) is fetched from the audit target instead,
/// since that domain may not serve this build yet.
fn rebase(u: &Url, origin: &Url, site_hosts: &BTreeSet<String>) -> Url {
    let same = authority(u) == authority(origin);
    if !same && authority(u).is_some_and(|a| site_hosts.contains(&a)) {
        if let Ok(r) = origin.join(&path_and_query(u)) {
            return r;
        }
    }
    u.clone()
}

fn link_result(fetched: &Result<Fetched, String>) -> LinkResult {
    match fetched {
        Ok(f) => {
            let indexable_page = f.status == 200
                && f.is_html()
                && !audit_page::is_noindex(
                    &audit_page::parse_page(&f.body),
                    f.x_robots_tag.as_deref(),
                );
            LinkResult {
                status: Some(f.status),
                indexable_page,
                error: None,
            }
        }
        Err(e) => LinkResult {
            status: None,
            indexable_page: false,
            error: Some(e.clone()),
        },
    }
}

/// None when `url` returns 200 with an image content type; else what is wrong.
fn check_image(agent: &ureq::Agent, url: &Url) -> Option<String> {
    // Only the status and headers matter; read none of the body.
    match fetch(agent, url, 0) {
        Err(e) => Some(format!("does not answer ({e})")),
        Ok(f) if f.status != 200 => Some(format!("returns {}", f.status)),
        Ok(f) => match f.content_type.as_deref() {
            Some(ct) if ct.to_ascii_lowercase().starts_with("image/") => None,
            Some(ct) => Some(format!("returns {ct}, not an image")),
            None => Some("returns no content type".into()),
        },
    }
}

fn run_lighthouse_pages(
    opts: &Options,
    pages: &mut [PageState],
    progress: &dyn Fn(&str),
) -> LighthouseRun {
    let mut run = LighthouseRun {
        ran: false,
        version: LIGHTHOUSE_VERSION,
        chrome: None,
        pages: 0,
        skipped: None,
    };
    if opts.lighthouse_pages == 0 {
        run.skipped = Some("turned off (--no-lighthouse or --lighthouse-pages 0)".into());
        return run;
    }
    let Some(chrome) = audit_lighthouse::find_chrome() else {
        run.skipped = Some(
            "no Chrome or Chromium found. Set CHROME_PATH, or put chromium or google-chrome on PATH."
                .into(),
        );
        return run;
    };
    run.chrome = Some(chrome.display().to_string());
    if !audit_lighthouse::bun_available() {
        run.skipped = Some("Bun is not installed. Lighthouse runs through `bun x`.".into());
        return run;
    }
    let eligible = pages
        .iter_mut()
        .filter(|p| p.facts.is_some() && p.fetched.as_ref().is_ok_and(|f| f.status == 200));
    for page in eligible.take(opts.lighthouse_pages) {
        let Ok(f) = &page.fetched else { continue };
        progress(&format!("Lighthouse {}", page.path));
        match audit_lighthouse::run_lighthouse(f.url.as_str(), &chrome) {
            Ok(report) => {
                let (scores, issues) = audit_lighthouse::interpret(&report);
                page.lighthouse = Some(scores);
                page.issues.extend(issues);
                run.ran = true;
                run.pages += 1;
            }
            Err(e) => {
                // Chrome that fails once fails the same way on every page.
                run.skipped = Some(format!("stopped after a failed run on {}: {e}", page.path));
                break;
            }
        }
    }
    run
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

fn print_report(report: &AuditReport) {
    let color = output::use_color();
    println!();
    println!("Audit of {}", report.target);
    let source = match report.page_source {
        "sitemap" => "the sitemap",
        "routes" => "the app routes",
        _ => "the home page links",
    };
    print!("{} page(s) from {source}", report.pages_audited);
    if report.pages_found > report.pages_audited {
        print!(
            ", {} more not audited (raise --max-pages)",
            report.pages_found - report.pages_audited
        );
    }
    println!();

    if !report.site.is_empty() {
        println!();
        println!("Site");
        for d in &report.site {
            print!("{}", output::format_diagnostic(d, color));
        }
    }
    for page in &report.pages {
        println!();
        let status = page
            .status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "no response".into());
        let scores = page.lighthouse.as_ref().map(|s| {
            let mut parts = Vec::new();
            if let Some(v) = s.performance {
                parts.push(format!("performance {v}"));
            }
            if let Some(v) = s.accessibility {
                parts.push(format!("accessibility {v}"));
            }
            parts.join(", ")
        });
        let mut header = format!("{}  {status}", page.path);
        if let Some(s) = scores.filter(|s| !s.is_empty()) {
            header.push_str(&format!("  {s}"));
        }
        if page.issues.is_empty() {
            println!("{header}  no problems");
            continue;
        }
        println!("{header}");
        for d in &page.issues {
            print!("{}", output::format_diagnostic(d, color));
        }
    }
    println!();
    match (&report.lighthouse.skipped, report.lighthouse.ran) {
        (None, true) => println!(
            "Lighthouse {} ran on {} page(s) with {}.",
            report.lighthouse.version,
            report.lighthouse.pages,
            report.lighthouse.chrome.as_deref().unwrap_or("Chrome")
        ),
        (Some(why), true) => println!(
            "Lighthouse ran on {} page(s), then {why}",
            report.lighthouse.pages
        ),
        (Some(why), false) => println!("Lighthouse did not run: {why}"),
        (None, false) => println!("Lighthouse did not run: no page returned 200 HTML."),
    }
    println!();
    if report.errors == 0 {
        println!(
            "✓ audit passed: 0 errors, {} warning(s) on {} page(s)",
            report.warnings, report.pages_audited
        );
    } else {
        println!(
            "✗ audit failed: {} error(s), {} warning(s) on {} page(s)",
            report.errors, report.warnings, report.pages_audited
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn target_forms() {
        assert_eq!(
            parse_target("https://x.example").unwrap().as_str(),
            "https://x.example/"
        );
        assert_eq!(
            parse_target("x.example/a").unwrap().as_str(),
            "https://x.example/a"
        );
        assert_eq!(
            parse_target("localhost:3000").unwrap().as_str(),
            "http://localhost:3000/"
        );
        assert_eq!(
            parse_target("127.0.0.1:3000").unwrap().as_str(),
            "http://127.0.0.1:3000/"
        );
        assert!(parse_target("ftp://x.example").is_err());
    }

    #[test]
    fn options() {
        let o = parse_options(&s(&[
            "audit",
            "https://x.example",
            "--max-pages",
            "5",
            "--no-lighthouse",
        ]))
        .unwrap();
        assert_eq!(o.target.as_str(), "https://x.example/");
        assert_eq!(o.max_pages, 5);
        assert_eq!(o.lighthouse_pages, 0);

        let o = parse_options(&s(&[
            "audit",
            "--lighthouse-pages=1",
            "--url",
            "localhost:9",
        ]))
        .unwrap();
        assert_eq!(o.target.as_str(), "http://localhost:9/");
        assert_eq!(o.lighthouse_pages, 1);

        let o = parse_options(&s(&["audit", "x.example"])).unwrap();
        assert_eq!(o.max_pages, DEFAULT_MAX_PAGES);
        assert_eq!(o.lighthouse_pages, DEFAULT_LIGHTHOUSE_PAGES);

        assert!(parse_options(&s(&["audit", "x.example", "--max-pages", "0"])).is_err());
        assert!(parse_options(&s(&["audit", "x.example", "--max-pages", "lots"])).is_err());
    }

    #[test]
    fn rebase_moves_only_site_hosts() {
        let origin = Url::parse("http://localhost:4321/").unwrap();
        let hosts: BTreeSet<String> = [
            "localhost:4321".to_string(),
            "rosas.example:443".to_string(),
        ]
        .into_iter()
        .collect();
        let prod = Url::parse("https://rosas.example/menu?x=1").unwrap();
        assert_eq!(
            rebase(&prod, &origin, &hosts).as_str(),
            "http://localhost:4321/menu?x=1"
        );
        let cdn = Url::parse("https://cdn.example/a.png").unwrap();
        assert_eq!(rebase(&cdn, &origin, &hosts), cdn);
    }

    #[test]
    fn manifest_routes_keep_static_pages_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pylon.manifest.json");
        let manifest = serde_json::json!({
            "manifest_version": 1, "name": "t", "version": "0.1.0",
            "entities": [], "queries": [], "actions": [], "policies": [],
            "routes": [
                { "path": "/", "mode": "ssr", "component": "app/page" },
                { "path": "/about", "mode": "ssr", "component": "app/about/page" },
                { "path": "/blog/:slug", "mode": "ssr", "component": "app/blog/[slug]/page" },
                { "path": "/sitemap.xml", "mode": "ssr", "component": "app/sitemap", "kind": "sitemap" },
                { "path": "/opengraph-image", "mode": "ssr", "component": "app/opengraph-image", "kind": "og-image" },
                { "path": "/", "mode": "ssr", "component": "app/not-found", "kind": "not-found" }
            ]
        });
        std::fs::write(&path, manifest.to_string()).unwrap();
        assert_eq!(manifest_page_routes(&path), vec!["/", "/about"]);
        assert!(manifest_page_routes(&dir.path().join("missing.json")).is_empty());
    }
}
