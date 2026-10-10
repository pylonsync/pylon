//! Page checks for `pylon audit`.
//!
//! Everything here is pure: it takes HTML (parsed with html5ever through the
//! `scraper` crate) and facts about the page, and returns diagnostics. The
//! network side lives in `audit.rs`. Each diagnostic's `hint` is the fix, in
//! one line.

use std::collections::BTreeMap;

use pylon_kernel::{Diagnostic, Severity};
use scraper::{Html, Selector};
use url::Url;

/// Title length limits in characters. Google cuts a title off at about 600
/// pixels, which is about 60 characters.
pub const TITLE_MIN: usize = 10;
pub const TITLE_MAX: usize = 60;
/// Description length limits in characters. Google shows about 155 to 160.
pub const DESCRIPTION_MIN: usize = 50;
pub const DESCRIPTION_MAX: usize = 160;

/// One `<script type="application/ld+json">` block.
#[derive(Debug, Clone)]
pub struct JsonLdBlock {
    pub value: Result<serde_json::Value, String>,
}

/// What the checks need from one HTML document.
#[derive(Debug, Clone, Default)]
pub struct PageFacts {
    /// `<html lang>`, when the attribute is present.
    pub lang: Option<String>,
    /// Text of every `<title>` in `<head>`, whitespace collapsed.
    pub titles: Vec<String>,
    /// `content` of every `<meta name="description">`.
    pub descriptions: Vec<String>,
    pub h1_count: usize,
    /// `href` of every `<link rel="canonical">`.
    pub canonicals: Vec<String>,
    /// `content` of `<meta name="viewport">`.
    pub viewport: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    /// `content` of the first `og:image` (or `og:image:url`) meta tag.
    pub og_image: Option<String>,
    /// `content` of every `<meta name="robots">` and `<meta name="googlebot">`.
    pub robots: Vec<String>,
    /// `src` of each `<img>` without an `alt` attribute. `alt=""` is valid
    /// for a decorative image, so only a missing attribute counts.
    pub images_missing_alt: Vec<String>,
    pub json_ld: Vec<JsonLdBlock>,
    /// `href` of every `<a>`, resolved later against the page URL.
    pub links: Vec<String>,
    /// `<base href>`, which changes how relative links resolve.
    pub base_href: Option<String>,
}

fn selector(css: &str) -> Selector {
    // The selectors below are constants; a parse failure is a bug here.
    Selector::parse(css).unwrap_or_else(|e| panic!("invalid selector {css:?}: {e}"))
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse an HTML document into the facts the checks read.
pub fn parse_page(html: &str) -> PageFacts {
    let doc = Html::parse_document(html);
    let mut facts = PageFacts::default();

    if let Some(root) = doc.select(&selector("html")).next() {
        facts.lang = root.value().attr("lang").map(|s| s.trim().to_string());
    }
    for el in doc.select(&selector("html > head > title")) {
        facts
            .titles
            .push(collapse_ws(&el.text().collect::<String>()));
    }
    for el in doc.select(&selector("meta")) {
        let v = el.value();
        let content = v.attr("content").map(|s| s.trim().to_string());
        let name = v.attr("name").map(str::to_ascii_lowercase);
        let property = v.attr("property").map(str::to_ascii_lowercase);
        // Open Graph tags use `property`; some sites use `name`. Accept both.
        let og_key = property
            .clone()
            .or_else(|| name.clone())
            .unwrap_or_default();
        match name.as_deref() {
            Some("description") => facts.descriptions.push(content.clone().unwrap_or_default()),
            Some("viewport") if facts.viewport.is_none() => facts.viewport = content.clone(),
            Some("robots") | Some("googlebot") => {
                facts.robots.push(content.clone().unwrap_or_default())
            }
            _ => {}
        }
        match og_key.as_str() {
            "og:title" if facts.og_title.is_none() => facts.og_title = content,
            "og:description" if facts.og_description.is_none() => facts.og_description = content,
            "og:image" | "og:image:url" if facts.og_image.is_none() => facts.og_image = content,
            _ => {}
        }
    }
    for el in doc.select(&selector("link[href]")) {
        let rel = el.value().attr("rel").unwrap_or_default();
        if rel
            .split_ascii_whitespace()
            .any(|r| r.eq_ignore_ascii_case("canonical"))
        {
            facts.canonicals.push(
                el.value()
                    .attr("href")
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            );
        }
    }
    facts.h1_count = doc.select(&selector("h1")).count();
    for el in doc.select(&selector("img")) {
        if el.value().attr("alt").is_none() {
            facts
                .images_missing_alt
                .push(el.value().attr("src").unwrap_or("(no src)").to_string());
        }
    }
    for el in doc.select(&selector("script[type]")) {
        let ty = el.value().attr("type").unwrap_or_default();
        if !ty.trim().eq_ignore_ascii_case("application/ld+json") {
            continue;
        }
        let text = el.text().collect::<String>();
        let value = serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string());
        facts.json_ld.push(JsonLdBlock { value });
    }
    for el in doc.select(&selector("a[href]")) {
        if let Some(href) = el.value().attr("href") {
            facts.links.push(href.trim().to_string());
        }
    }
    if let Some(base) = doc.select(&selector("head > base[href]")).next() {
        facts.base_href = base.value().attr("href").map(|s| s.trim().to_string());
    }
    facts
}

/// True when a `<meta name="robots">` or an `X-Robots-Tag` header says
/// `noindex` (or `none`, which means noindex and nofollow).
pub fn is_noindex(facts: &PageFacts, x_robots_tag: Option<&str>) -> bool {
    let says_noindex = |s: &str| {
        s.split(|c: char| c == ',' || c.is_whitespace())
            .map(|t| t.trim().to_ascii_lowercase())
            .any(|t| t == "noindex" || t == "none")
    };
    facts.robots.iter().any(|r| says_noindex(r)) || x_robots_tag.is_some_and(says_noindex)
}

/// Every `@type` in a JSON-LD value, including nested nodes and `@graph`.
pub fn json_ld_types(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            match map.get("@type") {
                Some(serde_json::Value::String(t)) => out.push(t.clone()),
                Some(serde_json::Value::Array(ts)) => {
                    out.extend(ts.iter().filter_map(|t| t.as_str().map(str::to_string)))
                }
                _ => {}
            }
            for (k, v) in map {
                if k != "@type" {
                    json_ld_types(v, out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                json_ld_types(item, out);
            }
        }
        _ => {}
    }
}

/// True when `ty` is schema.org Organization or one of its subtypes
/// (LocalBusiness, Restaurant, Dentist, ...). Accepts `schema:X` and full
/// schema.org URLs as well as the bare name.
pub fn is_organization_type(ty: &str) -> bool {
    let name = ty
        .trim()
        .trim_start_matches("https://schema.org/")
        .trim_start_matches("http://schema.org/")
        .trim_start_matches("schema:");
    super::audit_org_types::ORGANIZATION_TYPES
        .binary_search(&name)
        .is_ok()
}

/// Facts about the page that do not come from its HTML.
#[derive(Debug, Clone)]
pub struct PageContext<'a> {
    /// The URL the page was fetched from (after redirects).
    pub url: &'a Url,
    pub is_home: bool,
    pub in_sitemap: bool,
    pub x_robots_tag: Option<&'a str>,
    /// True when the audit target is on this machine. Absolute URLs that
    /// point at localhost are only a problem on a deployed site.
    pub target_is_local: bool,
}

pub fn diag(severity: Severity, code: &str, message: String, fix: &str) -> Diagnostic {
    Diagnostic {
        severity,
        code: code.into(),
        message,
        span: None,
        hint: Some(fix.into()),
    }
}

fn error(code: &str, message: impl Into<String>, fix: &str) -> Diagnostic {
    diag(Severity::Error, code, message.into(), fix)
}

fn warning(code: &str, message: impl Into<String>, fix: &str) -> Diagnostic {
    diag(Severity::Warning, code, message.into(), fix)
}

/// True for localhost, 127.0.0.0/8, and ::1.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The checks that read one page on its own. Uniqueness across pages is in
/// [`check_duplicates`].
pub fn check_page(facts: &PageFacts, ctx: &PageContext) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let noindex = is_noindex(facts, ctx.x_robots_tag);

    if noindex && ctx.in_sitemap {
        out.push(error(
            "AUDIT_NOINDEX_IN_SITEMAP",
            "The page is in the sitemap, but it has noindex.",
            "Remove noindex from the page, or remove the page from the sitemap.",
        ));
    }

    // Title.
    match facts.titles.first() {
        None => out.push(error(
            "AUDIT_TITLE_MISSING",
            "The page has no <title>.",
            "Set `title` in the page's `metadata` export (or in generateMetadata).",
        )),
        Some(t) if t.is_empty() => out.push(error(
            "AUDIT_TITLE_MISSING",
            "The <title> is empty.",
            "Set `title` in the page's `metadata` export (or in generateMetadata).",
        )),
        Some(t) => {
            let len = t.chars().count();
            if len < TITLE_MIN {
                out.push(warning(
                    "AUDIT_TITLE_SHORT",
                    format!("The title \"{t}\" has {len} characters."),
                    "Write a title of 10 to 60 characters that names the page and the business.",
                ));
            } else if len > TITLE_MAX {
                out.push(warning(
                    "AUDIT_TITLE_LONG",
                    format!("The title has {len} characters. Google cuts it off at about 60."),
                    "Shorten the title to 60 characters or fewer.",
                ));
            }
        }
    }
    if facts.titles.len() > 1 {
        out.push(warning(
            "AUDIT_TITLE_MULTIPLE",
            format!("The page has {} <title> tags.", facts.titles.len()),
            "Keep one <title>. Set it only through `metadata`, not in a layout and a page.",
        ));
    }

    // Description.
    match facts.descriptions.first() {
        None => out.push(error(
            "AUDIT_DESCRIPTION_MISSING",
            "The page has no meta description.",
            "Set `description` in the page's `metadata` export.",
        )),
        Some(d) if d.is_empty() => out.push(error(
            "AUDIT_DESCRIPTION_MISSING",
            "The meta description is empty.",
            "Set `description` in the page's `metadata` export.",
        )),
        Some(d) => {
            let len = d.chars().count();
            if len < DESCRIPTION_MIN {
                out.push(warning(
                    "AUDIT_DESCRIPTION_SHORT",
                    format!("The meta description has {len} characters."),
                    "Write a description of 50 to 160 characters that says what the page offers.",
                ));
            } else if len > DESCRIPTION_MAX {
                out.push(warning(
                    "AUDIT_DESCRIPTION_LONG",
                    format!("The meta description has {len} characters. Google cuts it off at about 160."),
                    "Shorten the description to 160 characters or fewer.",
                ));
            }
        }
    }
    if facts.descriptions.len() > 1 {
        out.push(warning(
            "AUDIT_DESCRIPTION_MULTIPLE",
            format!(
                "The page has {} meta descriptions.",
                facts.descriptions.len()
            ),
            "Keep one meta description.",
        ));
    }

    // Headings.
    match facts.h1_count {
        0 => out.push(error(
            "AUDIT_H1_MISSING",
            "The page has no <h1>.",
            "Add one <h1> that states the main topic of the page.",
        )),
        1 => {}
        n => out.push(warning(
            "AUDIT_H1_MULTIPLE",
            format!("The page has {n} <h1> elements."),
            "Keep one <h1>. Use <h2> and lower for the sections.",
        )),
    }

    // Canonical.
    match facts.canonicals.len() {
        0 => out.push(warning(
            "AUDIT_CANONICAL_MISSING",
            "The page has no canonical link.",
            "Set `canonical` in the page's `metadata` to the page's full URL.",
        )),
        1 => out.extend(check_canonical(&facts.canonicals[0], ctx)),
        n => out.push(error(
            "AUDIT_CANONICAL_MULTIPLE",
            format!("The page has {n} canonical links. Google ignores all of them."),
            "Keep one <link rel=\"canonical\">.",
        )),
    }

    // Language.
    match facts.lang.as_deref() {
        None | Some("") => out.push(error(
            "AUDIT_LANG_MISSING",
            "The <html> element has no lang attribute.",
            "Set <html lang=\"en\"> (or the page's language) in the root layout.",
        )),
        Some(lang) if !is_plausible_lang(lang) => out.push(warning(
            "AUDIT_LANG_INVALID",
            format!("lang=\"{lang}\" is not a language tag."),
            "Use a BCP 47 tag such as \"en\" or \"en-US\".",
        )),
        _ => {}
    }

    // Viewport.
    match facts.viewport.as_deref() {
        None => out.push(error(
            "AUDIT_VIEWPORT_MISSING",
            "The page has no viewport meta tag. Phones show it zoomed out.",
            "Add <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"> to the root layout.",
        )),
        Some(v) if !v.to_ascii_lowercase().contains("width=device-width") => out.push(warning(
            "AUDIT_VIEWPORT_WIDTH",
            format!("The viewport is \"{v}\", without width=device-width."),
            "Use content=\"width=device-width, initial-scale=1\".",
        )),
        _ => {}
    }

    // Open Graph.
    if facts.og_title.as_deref().is_none_or(str::is_empty) {
        out.push(warning(
            "AUDIT_OG_TITLE_MISSING",
            "The page has no og:title.",
            "Set `openGraph.title` in the page's `metadata`.",
        ));
    }
    if facts.og_description.as_deref().is_none_or(str::is_empty) {
        out.push(warning(
            "AUDIT_OG_DESCRIPTION_MISSING",
            "The page has no og:description.",
            "Set `openGraph.description` in the page's `metadata`.",
        ));
    }
    match facts.og_image.as_deref() {
        None | Some("") => out.push(error(
            "AUDIT_OG_IMAGE_MISSING",
            "The page has no og:image. Shared links show no picture.",
            "Add app/opengraph-image.tsx (or opengraph-image.png next to the page); Pylon adds og:image.",
        )),
        Some(img) => match Url::parse(img) {
            Ok(u) if u.scheme() == "http" || u.scheme() == "https" => {
                if !ctx.target_is_local && u.host_str().is_some_and(is_loopback_host) {
                    out.push(error(
                        "AUDIT_OG_IMAGE_LOCALHOST",
                        format!("og:image points at {img}."),
                        "Build the image URL from the public origin, not localhost.",
                    ));
                }
            }
            _ => out.push(error(
                "AUDIT_OG_IMAGE_RELATIVE",
                format!("og:image \"{img}\" is not an absolute http(s) URL. X and LinkedIn do not resolve it."),
                "Use an absolute URL, or use the opengraph-image file convention.",
            )),
        },
    }

    // Images.
    if !facts.images_missing_alt.is_empty() {
        let n = facts.images_missing_alt.len();
        let sample = facts
            .images_missing_alt
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        out.push(error(
            "AUDIT_IMG_ALT_MISSING",
            format!("{n} image(s) have no alt attribute: {sample}."),
            "Give each <img> an alt that describes it, or alt=\"\" if it is decoration.",
        ));
    }

    // Structured data.
    let mut types = Vec::new();
    for (i, block) in facts.json_ld.iter().enumerate() {
        match &block.value {
            Ok(v) => json_ld_types(v, &mut types),
            Err(e) => out.push(error(
                "AUDIT_JSONLD_INVALID",
                format!("JSON-LD block {} is not valid JSON: {e}.", i + 1),
                "Build the block with JSON.stringify(object), not by hand.",
            )),
        }
    }
    if ctx.is_home {
        if facts.json_ld.is_empty() {
            out.push(error(
                "AUDIT_JSONLD_MISSING",
                "The home page has no structured data (JSON-LD).",
                "Add a <script type=\"application/ld+json\"> with an Organization or LocalBusiness (name, url, logo, address, telephone).",
            ));
        } else if !types.iter().any(|t| is_organization_type(t)) {
            out.push(warning(
                "AUDIT_JSONLD_NO_ORGANIZATION",
                "The home page's structured data has no Organization or LocalBusiness.",
                "Add an Organization or LocalBusiness node (or a subtype such as Restaurant) to the home page JSON-LD.",
            ));
        }
    }

    out
}

fn check_canonical(href: &str, ctx: &PageContext) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    if href.is_empty() {
        out.push(error(
            "AUDIT_CANONICAL_INVALID",
            "The canonical link has an empty href.",
            "Set `canonical` in the page's `metadata` to the page's full URL.",
        ));
        return out;
    }
    let absolute = match Url::parse(href) {
        Ok(u) => u,
        Err(_) => {
            out.push(warning(
                "AUDIT_CANONICAL_RELATIVE",
                format!("The canonical link \"{href}\" is relative."),
                "Use the page's full URL, with https:// and the domain.",
            ));
            match ctx.url.join(href) {
                Ok(u) => u,
                Err(_) => return out,
            }
        }
    };
    if !ctx.target_is_local && absolute.host_str().is_some_and(is_loopback_host) {
        out.push(error(
            "AUDIT_CANONICAL_LOCALHOST",
            format!("The canonical link points at {href}."),
            "Build the canonical URL from the public origin, not localhost.",
        ));
    }
    // The canonical host is usually the production domain, which differs
    // from a local audit target, so only the paths are compared.
    if normalize_path(absolute.path()) != normalize_path(ctx.url.path()) {
        out.push(warning(
            "AUDIT_CANONICAL_OTHER_PAGE",
            format!(
                "The canonical link points at {}, not at this page ({}).",
                absolute.path(),
                ctx.url.path()
            ),
            "Point the canonical at this page, unless this page is a copy of that one.",
        ));
    }
    out
}

/// `/about/` and `/about` are the same page for this comparison.
pub fn normalize_path(path: &str) -> &str {
    if path.len() > 1 {
        path.trim_end_matches('/')
    } else {
        path
    }
}

fn is_plausible_lang(lang: &str) -> bool {
    let mut parts = lang.split(['-', '_']);
    let primary = parts.next().unwrap_or_default();
    (2..=3).contains(&primary.len())
        && primary.chars().all(|c| c.is_ascii_alphabetic())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// Duplicate titles and descriptions across pages. `pages` maps a page path
/// to its facts; noindex pages are left out by the caller. Returns the
/// diagnostics to add, by page path.
pub fn check_duplicates(pages: &[(String, &PageFacts)]) -> BTreeMap<String, Vec<Diagnostic>> {
    let mut by_title: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut by_desc: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (path, facts) in pages {
        if let Some(t) = facts.titles.first().filter(|t| !t.is_empty()) {
            by_title.entry(t.as_str()).or_default().push(path.as_str());
        }
        if let Some(d) = facts.descriptions.first().filter(|d| !d.is_empty()) {
            by_desc.entry(d.as_str()).or_default().push(path.as_str());
        }
    }
    let mut out: BTreeMap<String, Vec<Diagnostic>> = BTreeMap::new();
    for paths in by_title.values().filter(|p| p.len() > 1) {
        for &path in paths {
            let others = others_list(paths, path);
            out.entry(path.to_string()).or_default().push(error(
                "AUDIT_TITLE_DUPLICATE",
                format!("The title is the same as on {others}."),
                "Give each page its own title that names what is on it.",
            ));
        }
    }
    for paths in by_desc.values().filter(|p| p.len() > 1) {
        for &path in paths {
            let others = others_list(paths, path);
            out.entry(path.to_string()).or_default().push(warning(
                "AUDIT_DESCRIPTION_DUPLICATE",
                format!("The meta description is the same as on {others}."),
                "Write a description for each page about what is on it.",
            ));
        }
    }
    out
}

fn others_list(paths: &[&str], current: &str) -> String {
    let others: Vec<&str> = paths.iter().copied().filter(|p| *p != current).collect();
    if others.len() > 3 {
        format!("{} and {} more", others[..3].join(", "), others.len() - 3)
    } else {
        others.join(", ")
    }
}

/// Resolve a page's `<a href>` values to absolute http(s) URLs without
/// fragments. mailto:, tel:, javascript: and the like are dropped.
pub fn resolve_links(facts: &PageFacts, page_url: &Url) -> Vec<Url> {
    let base = facts
        .base_href
        .as_deref()
        .and_then(|b| page_url.join(b).ok())
        .unwrap_or_else(|| page_url.clone());
    let mut out = Vec::new();
    for href in &facts.links {
        if href.is_empty() || href.starts_with('#') {
            continue;
        }
        let Ok(mut u) = base.join(href) else {
            continue;
        };
        if u.scheme() != "http" && u.scheme() != "https" {
            continue;
        }
        u.set_fragment(None);
        out.push(u);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_HOME: &str = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Rosa's Bakery | Fresh bread in Austin</title>
  <meta name="description" content="Rosa's Bakery bakes sourdough, pastries and custom cakes every morning in East Austin. Order online for pickup.">
  <link rel="canonical" href="https://rosas.example/">
  <meta property="og:title" content="Rosa's Bakery">
  <meta property="og:description" content="Fresh bread in Austin.">
  <meta property="og:image" content="https://rosas.example/opengraph-image">
  <script type="application/ld+json">{"@context":"https://schema.org","@type":"Bakery","name":"Rosa's Bakery"}</script>
</head>
<body>
  <h1>Rosa's Bakery</h1>
  <img src="/logo.png" alt="Rosa's Bakery logo">
  <img src="/divider.png" alt="">
  <a href="/menu">Menu</a>
  <a href="https://rosas.example/about#team">About</a>
  <a href="mailto:hi@rosas.example">Email</a>
  <a href="#top">Top</a>
  <a href="https://other.example/">Other</a>
</body>
</html>"##;

    const BAD_PAGE: &str = r#"<html>
<head>
  <script type="application/ld+json">{"@type": "Organization",}</script>
  <link rel="canonical" href="/x"><link rel="canonical" href="/y">
</head>
<body>
  <h1>One</h1><h1>Two</h1>
  <img src="/a.png"><img src="/b.png"><img src="/c.png"><img src="/d.png">
  <svg><title>icon</title></svg>
</body>
</html>"#;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn ctx<'a>(u: &'a Url, is_home: bool) -> PageContext<'a> {
        PageContext {
            url: u,
            is_home,
            in_sitemap: true,
            x_robots_tag: None,
            target_is_local: true,
        }
    }

    fn codes(d: &[Diagnostic]) -> Vec<&str> {
        d.iter().map(|d| d.code.as_str()).collect()
    }

    fn severity_of(d: &[Diagnostic], code: &str) -> Severity {
        d.iter()
            .find(|d| d.code == code)
            .unwrap_or_else(|| panic!("no {code} in {:?}", codes(d)))
            .severity
    }

    #[test]
    fn a_complete_home_page_passes() {
        let facts = parse_page(GOOD_HOME);
        let u = url("http://localhost:4321/");
        let d = check_page(&facts, &ctx(&u, true));
        assert!(d.is_empty(), "{:?}", codes(&d));
    }

    #[test]
    fn parse_reads_head_fields() {
        let f = parse_page(GOOD_HOME);
        assert_eq!(f.lang.as_deref(), Some("en"));
        assert_eq!(f.titles, vec!["Rosa's Bakery | Fresh bread in Austin"]);
        assert_eq!(f.h1_count, 1);
        assert_eq!(f.canonicals, vec!["https://rosas.example/"]);
        assert_eq!(
            f.og_image.as_deref(),
            Some("https://rosas.example/opengraph-image")
        );
        assert!(
            f.images_missing_alt.is_empty(),
            "alt=\"\" counts as present"
        );
        assert_eq!(f.json_ld.len(), 1);
    }

    #[test]
    fn an_empty_page_fails_every_required_check() {
        let facts = parse_page("<html><body><p>hi</p></body></html>");
        let u = url("http://localhost:4321/");
        let d = check_page(&facts, &ctx(&u, true));
        for code in [
            "AUDIT_TITLE_MISSING",
            "AUDIT_DESCRIPTION_MISSING",
            "AUDIT_H1_MISSING",
            "AUDIT_LANG_MISSING",
            "AUDIT_VIEWPORT_MISSING",
            "AUDIT_OG_IMAGE_MISSING",
            "AUDIT_JSONLD_MISSING",
        ] {
            assert_eq!(severity_of(&d, code), Severity::Error, "{code}");
        }
        for code in [
            "AUDIT_CANONICAL_MISSING",
            "AUDIT_OG_TITLE_MISSING",
            "AUDIT_OG_DESCRIPTION_MISSING",
        ] {
            assert_eq!(severity_of(&d, code), Severity::Warning, "{code}");
        }
        assert!(d
            .iter()
            .all(|d| d.hint.as_deref().is_some_and(|h| !h.is_empty())));
    }

    #[test]
    fn bad_page_problems_are_found() {
        let facts = parse_page(BAD_PAGE);
        // The <title> inside <svg> is not the document title.
        assert!(facts.titles.is_empty());
        let u = url("http://localhost:4321/about");
        let d = check_page(&facts, &ctx(&u, false));
        assert_eq!(severity_of(&d, "AUDIT_JSONLD_INVALID"), Severity::Error);
        assert_eq!(severity_of(&d, "AUDIT_CANONICAL_MULTIPLE"), Severity::Error);
        assert_eq!(severity_of(&d, "AUDIT_H1_MULTIPLE"), Severity::Warning);
        let alt = d
            .iter()
            .find(|d| d.code == "AUDIT_IMG_ALT_MISSING")
            .unwrap();
        assert!(alt.message.starts_with("4 image(s)"), "{}", alt.message);
        assert!(
            alt.message.contains("/a.png, /b.png, /c.png."),
            "{}",
            alt.message
        );
        // Not the home page: no structured-data requirement.
        assert!(!codes(&d).contains(&"AUDIT_JSONLD_MISSING"));
    }

    #[test]
    fn title_and_description_lengths() {
        let long = "x".repeat(70);
        let html = format!(
            r#"<html lang="en"><head><title>Hi</title><meta name="description" content="{long}{long}{long}"></head></html>"#
        );
        let f = parse_page(&html);
        let u = url("http://localhost/");
        let d = check_page(&f, &ctx(&u, false));
        assert_eq!(severity_of(&d, "AUDIT_TITLE_SHORT"), Severity::Warning);
        assert_eq!(severity_of(&d, "AUDIT_DESCRIPTION_LONG"), Severity::Warning);

        let html = format!(
            r#"<html><head><title>{long}</title><meta name="description" content="Short."></head></html>"#
        );
        let f = parse_page(&html);
        let d = check_page(&f, &ctx(&u, false));
        assert_eq!(severity_of(&d, "AUDIT_TITLE_LONG"), Severity::Warning);
        assert_eq!(
            severity_of(&d, "AUDIT_DESCRIPTION_SHORT"),
            Severity::Warning
        );
    }

    #[test]
    fn noindex_in_sitemap_is_an_error() {
        let f = parse_page(
            r#"<html><head><meta name="robots" content="noindex, follow"></head></html>"#,
        );
        let u = url("http://localhost/x");
        let d = check_page(&f, &ctx(&u, false));
        assert_eq!(severity_of(&d, "AUDIT_NOINDEX_IN_SITEMAP"), Severity::Error);

        let mut c = ctx(&u, false);
        c.in_sitemap = false;
        assert!(!codes(&check_page(&f, &c)).contains(&"AUDIT_NOINDEX_IN_SITEMAP"));
    }

    #[test]
    fn x_robots_tag_header_counts_as_noindex() {
        let f = PageFacts::default();
        assert!(is_noindex(&f, Some("noindex")));
        assert!(is_noindex(&f, Some("none")));
        assert!(!is_noindex(&f, Some("nofollow")));
        assert!(!is_noindex(&f, None));
    }

    #[test]
    fn canonical_rules() {
        let u = url("http://localhost:4321/menu");
        let f = parse_page(r#"<link rel="canonical" href="/menu">"#);
        let d = check_page(&f, &ctx(&u, false));
        assert_eq!(
            severity_of(&d, "AUDIT_CANONICAL_RELATIVE"),
            Severity::Warning
        );
        assert!(!codes(&d).contains(&"AUDIT_CANONICAL_OTHER_PAGE"));

        // The production domain on a local audit is fine; a trailing slash
        // is the same page.
        let f = parse_page(r#"<link rel="canonical" href="https://rosas.example/menu/">"#);
        let d = check_page(&f, &ctx(&u, false));
        assert!(
            !codes(&d).iter().any(|c| c.starts_with("AUDIT_CANONICAL")),
            "{:?}",
            codes(&d)
        );

        let f = parse_page(r#"<link rel="canonical" href="https://rosas.example/">"#);
        let d = check_page(&f, &ctx(&u, false));
        assert_eq!(
            severity_of(&d, "AUDIT_CANONICAL_OTHER_PAGE"),
            Severity::Warning
        );

        // localhost in a canonical is only wrong on a deployed site.
        let deployed = url("https://rosas.example/menu");
        let f = parse_page(r#"<link rel="canonical" href="http://localhost:4321/menu">"#);
        let mut c = ctx(&deployed, false);
        c.target_is_local = false;
        let d = check_page(&f, &c);
        assert_eq!(
            severity_of(&d, "AUDIT_CANONICAL_LOCALHOST"),
            Severity::Error
        );
    }

    #[test]
    fn og_image_must_be_absolute() {
        let u = url("http://localhost/");
        let f = parse_page(r#"<meta property="og:image" content="/og.png">"#);
        let d = check_page(&f, &ctx(&u, false));
        assert_eq!(severity_of(&d, "AUDIT_OG_IMAGE_RELATIVE"), Severity::Error);
    }

    #[test]
    fn home_page_structured_data() {
        let u = url("http://localhost/");
        let website_only = parse_page(
            r#"<script type="application/ld+json">{"@type":"WebSite","name":"x"}</script>"#,
        );
        let d = check_page(&website_only, &ctx(&u, true));
        assert_eq!(
            severity_of(&d, "AUDIT_JSONLD_NO_ORGANIZATION"),
            Severity::Warning
        );

        let graph = parse_page(
            r#"<script type="application/ld+json">{"@context":"https://schema.org","@graph":[{"@type":"WebSite"},{"@type":["Thing","schema:Dentist"]}]}</script>"#,
        );
        let d = check_page(&graph, &ctx(&u, true));
        assert!(
            !codes(&d).iter().any(|c| c.starts_with("AUDIT_JSONLD")),
            "{:?}",
            codes(&d)
        );
    }

    #[test]
    fn organization_types_include_local_business_subtypes() {
        for t in [
            "Organization",
            "LocalBusiness",
            "Restaurant",
            "Plumber",
            "https://schema.org/Corporation",
        ] {
            assert!(is_organization_type(t), "{t}");
        }
        for t in ["WebSite", "Person", "Place", "Product"] {
            assert!(!is_organization_type(t), "{t}");
        }
    }

    #[test]
    fn org_type_table_is_sorted_for_binary_search() {
        let types = super::super::audit_org_types::ORGANIZATION_TYPES;
        assert!(types.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn duplicates_across_pages() {
        let a = parse_page(
            r#"<title>Same title here</title><meta name="description" content="Same description">"#,
        );
        let b = a.clone();
        let c = parse_page(
            r#"<title>Different</title><meta name="description" content="Same description">"#,
        );
        let pages = vec![
            ("/a".to_string(), &a),
            ("/b".to_string(), &b),
            ("/c".to_string(), &c),
        ];
        let d = check_duplicates(&pages);
        assert_eq!(
            severity_of(&d["/a"], "AUDIT_TITLE_DUPLICATE"),
            Severity::Error
        );
        assert!(d["/a"].iter().any(|x| x.message.contains("/b")));
        assert_eq!(
            severity_of(&d["/c"], "AUDIT_DESCRIPTION_DUPLICATE"),
            Severity::Warning
        );
        assert!(!codes(&d["/c"]).contains(&"AUDIT_TITLE_DUPLICATE"));
    }

    #[test]
    fn links_resolve_against_the_page_and_drop_non_http() {
        let f = parse_page(GOOD_HOME);
        let links = resolve_links(&f, &url("http://localhost:4321/"));
        let s: Vec<String> = links.iter().map(|u| u.to_string()).collect();
        assert_eq!(
            s,
            vec![
                "http://localhost:4321/menu",
                "https://rosas.example/about",
                "https://other.example/",
            ]
        );
    }

    #[test]
    fn base_href_changes_resolution() {
        let f = parse_page(r#"<head><base href="/blog/"></head><a href="post">x</a>"#);
        let links = resolve_links(&f, &url("http://localhost/"));
        assert_eq!(links[0].as_str(), "http://localhost/blog/post");
    }

    #[test]
    fn lang_tags() {
        assert!(is_plausible_lang("en"));
        assert!(is_plausible_lang("en-US"));
        assert!(is_plausible_lang("zh-Hant-TW"));
        assert!(!is_plausible_lang("english"));
        assert!(!is_plausible_lang("e"));
    }

    #[test]
    fn loopback_hosts() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("[::1]"));
        assert!(!is_loopback_host("rosas.example"));
    }
}
