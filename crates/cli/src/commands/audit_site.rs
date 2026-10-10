//! Site-level files for `pylon audit`: robots.txt and sitemap.xml.
//!
//! robots.txt follows RFC 9309 the way Google reads it: rules for a crawler
//! come from the group that names it, else from the `*` group; the longest
//! matching rule wins, and Allow wins a tie. sitemap.xml is read with an XML
//! parser (quick-xml), per https://www.sitemaps.org/protocol.html.

use pylon_kernel::{Diagnostic, Severity};
use quick_xml::events::Event;
use quick_xml::Reader;
use url::Url;

use super::audit_page::{diag, is_loopback_host};

/// The most URLs one sitemap file may list.
pub const SITEMAP_MAX_URLS: usize = 50_000;
const SITEMAP_NAMESPACE: &str = "http://www.sitemaps.org/schemas/sitemap/0.9";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotsRule {
    pub allow: bool,
    pub pattern: String,
}

#[derive(Debug, Clone, Default)]
pub struct RobotsGroup {
    /// User-agent tokens, lowercased.
    pub agents: Vec<String>,
    pub rules: Vec<RobotsRule>,
}

#[derive(Debug, Clone, Default)]
pub struct Robots {
    pub groups: Vec<RobotsGroup>,
    /// `Sitemap:` lines, as written.
    pub sitemaps: Vec<String>,
}

/// Parse robots.txt. Unknown lines are ignored, as crawlers do.
pub fn parse_robots(text: &str) -> Robots {
    let mut robots = Robots::default();
    // True while consecutive user-agent lines build up the current group's
    // agent list. A rule line ends that run.
    let mut in_agents = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "user-agent" => {
                if !in_agents {
                    robots.groups.push(RobotsGroup::default());
                    in_agents = true;
                }
                if let Some(group) = robots.groups.last_mut() {
                    group.agents.push(value.to_ascii_lowercase());
                }
            }
            "allow" | "disallow" => {
                in_agents = false;
                // A rule before any user-agent line belongs to no group.
                let Some(group) = robots.groups.last_mut() else {
                    continue;
                };
                // An empty Disallow allows everything; it adds no rule.
                if value.is_empty() {
                    continue;
                }
                group.rules.push(RobotsRule {
                    allow: key == "allow",
                    pattern: value.to_string(),
                });
            }
            "sitemap" => robots.sitemaps.push(value.to_string()),
            _ => {}
        }
    }
    robots
}

impl Robots {
    /// The rules that apply to `agent` (lowercase product token, such as
    /// "googlebot"): every group that names it, else every `*` group.
    fn rules_for(&self, agent: &str) -> Vec<&RobotsRule> {
        let groups_naming = |token: &str| {
            let token = token.to_string();
            self.groups
                .iter()
                .filter(move |g| g.agents.contains(&token))
        };
        let token = if groups_naming(agent).next().is_some() {
            agent
        } else {
            "*"
        };
        groups_naming(token).flat_map(|g| &g.rules).collect()
    }

    /// True when `agent` may fetch `path` (path plus optional query).
    pub fn is_allowed(&self, agent: &str, path: &str) -> bool {
        let mut best: Option<&RobotsRule> = None;
        for rule in self.rules_for(agent) {
            if !pattern_matches(&rule.pattern, path) {
                continue;
            }
            best = match best {
                None => Some(rule),
                Some(b) if rule.pattern.len() > b.pattern.len() => Some(rule),
                Some(b) if rule.pattern.len() == b.pattern.len() && rule.allow && !b.allow => {
                    Some(rule)
                }
                keep => keep,
            };
        }
        best.is_none_or(|r| r.allow)
    }
}

/// robots.txt path matching: `*` matches any run of characters, and a
/// trailing `$` anchors the end. Everything else is a prefix match.
pub fn pattern_matches(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(p) => (p, true),
        None => (pattern, false),
    };
    let parts: Vec<&str> = pattern.split('*').collect();
    // The first part must match at the start.
    let Some(mut rest) = path.strip_prefix(parts[0]) else {
        return false;
    };
    let last = parts.len() - 1;
    for (i, part) in parts.iter().enumerate().skip(1) {
        if i == last && anchored {
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    !anchored || rest.is_empty()
}

/// Problems in robots.txt itself. `paths` are the pages being audited, to
/// report the ones robots.txt blocks.
pub fn robots_diagnostics(robots: &Robots, sitemap_paths: &[String]) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    if !robots.is_allowed("googlebot", "/") {
        out.push(diag(
            Severity::Error,
            "AUDIT_ROBOTS_BLOCKS_ALL",
            "robots.txt blocks the whole site for search engines (Disallow: /).".into(),
            "Change app/robots.ts to allow \"/\" and disallow only private paths.",
        ));
    } else {
        let blocked: Vec<&str> = sitemap_paths
            .iter()
            .filter(|p| !robots.is_allowed("googlebot", p))
            .map(String::as_str)
            .collect();
        if !blocked.is_empty() {
            out.push(diag(
                Severity::Error,
                "AUDIT_ROBOTS_BLOCKS_SITEMAP_PAGE",
                format!(
                    "robots.txt blocks {} page(s) that the sitemap lists: {}.",
                    blocked.len(),
                    blocked
                        .iter()
                        .take(5)
                        .copied()
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                "Allow those paths in app/robots.ts, or remove them from the sitemap.",
            ));
        }
    }
    if robots.sitemaps.is_empty() {
        out.push(diag(
            Severity::Warning,
            "AUDIT_ROBOTS_NO_SITEMAP",
            "robots.txt has no Sitemap line.".into(),
            "Return `sitemap: \"https://<domain>/sitemap.xml\"` from app/robots.ts.",
        ));
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SitemapKind {
    UrlSet,
    Index,
}

/// One parsed sitemap file.
#[derive(Debug, Clone)]
pub struct SitemapDoc {
    pub kind: SitemapKind,
    /// `<loc>` values, as written.
    pub locs: Vec<String>,
    pub has_namespace: bool,
    /// `<url>` or `<sitemap>` entries with no `<loc>`.
    pub entries_without_loc: usize,
}

/// Parse a sitemap or sitemap index. Errors when the XML does not parse or
/// the root element is neither `<urlset>` nor `<sitemapindex>`.
pub fn parse_sitemap(xml: &str) -> Result<SitemapDoc, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut stack: Vec<String> = Vec::new();
    let mut doc: Option<SitemapDoc> = None;
    let mut current_loc: Option<String> = None;
    let mut entry_has_loc = false;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("XML error at byte {}: {e}", reader.error_position()))?;
        match event {
            // `<urlset/>`: a root with nothing inside it, and no End event.
            Event::Empty(e) if doc.is_none() => doc = Some(root_element(&e)?),
            Event::Start(e) if doc.is_none() => {
                doc = Some(root_element(&e)?);
                stack.push(String::from_utf8_lossy(e.local_name().as_ref()).into_owned());
            }
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if name == "url" || name == "sitemap" {
                    entry_has_loc = false;
                }
                if name == "loc" {
                    current_loc = Some(String::new());
                }
                stack.push(name);
            }
            Event::Empty(_) => {}
            Event::Text(t) => {
                if let Some(loc) = current_loc.as_mut() {
                    let text = t.xml10_content().map_err(|e| e.to_string())?;
                    loc.push_str(&text);
                }
            }
            Event::CData(t) => {
                if let Some(loc) = current_loc.as_mut() {
                    loc.push_str(&String::from_utf8_lossy(&t.into_inner()));
                }
            }
            Event::GeneralRef(r) => {
                if let Some(loc) = current_loc.as_mut() {
                    if let Some(ch) = r.resolve_char_ref().map_err(|e| e.to_string())? {
                        loc.push(ch);
                    } else {
                        let name = r.decode().map_err(|e| e.to_string())?;
                        match quick_xml::escape::resolve_predefined_entity(&name) {
                            Some(s) => loc.push_str(s),
                            None => return Err(format!("unknown entity &{name};")),
                        }
                    }
                }
            }
            Event::End(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                stack.pop();
                if name == "loc" {
                    if let (Some(loc), Some(d)) = (current_loc.take(), doc.as_mut()) {
                        let parent = stack.last().map(String::as_str);
                        let wanted = match d.kind {
                            SitemapKind::UrlSet => "url",
                            SitemapKind::Index => "sitemap",
                        };
                        if parent == Some(wanted) {
                            d.locs.push(loc.trim().to_string());
                            entry_has_loc = true;
                        }
                    }
                } else if (name == "url" || name == "sitemap") && !entry_has_loc {
                    if let Some(d) = doc.as_mut() {
                        d.entries_without_loc += 1;
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    doc.ok_or_else(|| "the file has no root element".into())
}

/// Read the root element: its kind and whether it declares the namespace.
fn root_element(e: &quick_xml::events::BytesStart) -> Result<SitemapDoc, String> {
    let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
    let kind = match name.as_str() {
        "urlset" => SitemapKind::UrlSet,
        "sitemapindex" => SitemapKind::Index,
        other => {
            return Err(format!(
                "the root element is <{other}>, not <urlset> or <sitemapindex>"
            ))
        }
    };
    let has_namespace = e
        .attributes()
        .flatten()
        .any(|a| a.key.as_ref() == b"xmlns" && a.value.as_ref() == SITEMAP_NAMESPACE.as_bytes());
    Ok(SitemapDoc {
        kind,
        locs: Vec::new(),
        has_namespace,
        entries_without_loc: 0,
    })
}

/// Problems in one parsed sitemap file at `path`.
pub fn sitemap_diagnostics(path: &str, doc: &SitemapDoc, target_is_local: bool) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    if doc.kind == SitemapKind::UrlSet && doc.locs.is_empty() {
        out.push(diag(
            Severity::Error,
            "AUDIT_SITEMAP_EMPTY",
            format!("{path} lists no pages."),
            "Return every public page from app/sitemap.ts.",
        ));
    }
    if !doc.has_namespace {
        out.push(diag(
            Severity::Warning,
            "AUDIT_SITEMAP_NAMESPACE",
            format!("The root element of {path} does not declare xmlns=\"{SITEMAP_NAMESPACE}\"."),
            "Generate the sitemap with app/sitemap.ts; Pylon writes the namespace.",
        ));
    }
    if doc.entries_without_loc > 0 {
        out.push(diag(
            Severity::Error,
            "AUDIT_SITEMAP_NO_LOC",
            format!(
                "{path} has {} entries without <loc>.",
                doc.entries_without_loc
            ),
            "Give each sitemap entry a `url`.",
        ));
    }
    if doc.locs.len() > SITEMAP_MAX_URLS {
        out.push(diag(
            Severity::Error,
            "AUDIT_SITEMAP_TOO_LARGE",
            format!("{path} lists {} URLs. The limit is 50,000.", doc.locs.len()),
            "Split the sitemap into several files under a sitemap index.",
        ));
    }
    let mut relative = Vec::new();
    let mut local = Vec::new();
    for loc in &doc.locs {
        match Url::parse(loc) {
            Ok(u) if u.scheme() == "http" || u.scheme() == "https" => {
                if !target_is_local && u.host_str().is_some_and(is_loopback_host) {
                    local.push(loc.as_str());
                }
            }
            _ => relative.push(loc.as_str()),
        }
    }
    if !relative.is_empty() {
        out.push(diag(
            Severity::Error,
            "AUDIT_SITEMAP_RELATIVE_URL",
            format!(
                "{path} has {} URL(s) that are not absolute: {}.",
                relative.len(),
                relative
                    .iter()
                    .take(3)
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "Write each sitemap URL in full, with https:// and the domain.",
        ));
    }
    if !local.is_empty() {
        out.push(diag(
            Severity::Error,
            "AUDIT_SITEMAP_LOCALHOST",
            format!(
                "{path} lists localhost URLs: {}.",
                local.iter().take(3).copied().collect::<Vec<_>>().join(", ")
            ),
            "Build sitemap URLs from the public domain, not from localhost.",
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_blocking_everything_is_found() {
        let r = parse_robots("User-agent: *\nDisallow: /\n");
        assert!(!r.is_allowed("googlebot", "/"));
        let d = robots_diagnostics(&r, &[]);
        assert_eq!(d[0].code, "AUDIT_ROBOTS_BLOCKS_ALL");
        assert_eq!(d[0].severity, Severity::Error);
        assert!(d.iter().any(|d| d.code == "AUDIT_ROBOTS_NO_SITEMAP"));
    }

    #[test]
    fn empty_disallow_allows_everything() {
        let r = parse_robots("User-agent: *\nDisallow:\nSitemap: https://x.example/sitemap.xml\n");
        assert!(r.is_allowed("googlebot", "/"));
        assert_eq!(r.sitemaps, vec!["https://x.example/sitemap.xml"]);
        assert!(robots_diagnostics(&r, &["/".into()]).is_empty());
    }

    #[test]
    fn a_named_group_replaces_the_star_group() {
        let r = parse_robots(
            "User-agent: *\nDisallow: /\n\nUser-agent: Googlebot\nAllow: /\nDisallow: /admin\n",
        );
        assert!(r.is_allowed("googlebot", "/"));
        assert!(!r.is_allowed("googlebot", "/admin/users"));
        assert!(!r.is_allowed("bingbot", "/"));
    }

    #[test]
    fn several_agents_share_one_group() {
        let r = parse_robots("User-agent: a\nUser-agent: *\nDisallow: /private\n");
        assert!(!r.is_allowed("googlebot", "/private/x"));
        assert!(r.is_allowed("googlebot", "/public"));
    }

    #[test]
    fn longest_match_wins_and_allow_wins_ties() {
        let r = parse_robots("User-agent: *\nDisallow: /shop\nAllow: /shop/public\n");
        assert!(r.is_allowed("googlebot", "/shop/public/item"));
        assert!(!r.is_allowed("googlebot", "/shop/cart"));
        let r = parse_robots("User-agent: *\nDisallow: /a\nAllow: /a\n");
        assert!(r.is_allowed("googlebot", "/a"));
    }

    #[test]
    fn wildcards_and_anchors() {
        assert!(pattern_matches("/*.pdf$", "/files/menu.pdf"));
        assert!(!pattern_matches("/*.pdf$", "/files/menu.pdf?x=1"));
        assert!(pattern_matches("/*?", "/search?q=1"));
        assert!(pattern_matches("/fish", "/fish.html"));
        assert!(!pattern_matches("/fish$", "/fish.html"));
        assert!(pattern_matches("/fish$", "/fish"));
        assert!(pattern_matches("/*", "/anything"));
        assert!(!pattern_matches("/a*b", "/acd"));
    }

    #[test]
    fn comments_and_case_are_ignored() {
        let r = parse_robots("# hi\nUSER-AGENT: *   # all\nDISALLOW: /tmp # temp\n");
        assert!(!r.is_allowed("googlebot", "/tmp/x"));
    }

    #[test]
    fn robots_blocking_a_sitemap_page_is_an_error() {
        let r = parse_robots("User-agent: *\nDisallow: /menu\nSitemap: https://x/s.xml");
        let d = robots_diagnostics(&r, &["/".into(), "/menu".into()]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "AUDIT_ROBOTS_BLOCKS_SITEMAP_PAGE");
        assert!(d[0].message.contains("/menu"));
    }

    const SITEMAP: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://rosas.example/</loc><priority>1</priority></url>
  <url><loc> https://rosas.example/menu?a=1&amp;b=2 </loc></url>
  <url><loc><![CDATA[https://rosas.example/about]]></loc></url>
</urlset>"#;

    #[test]
    fn parses_a_urlset() {
        let doc = parse_sitemap(SITEMAP).unwrap();
        assert_eq!(doc.kind, SitemapKind::UrlSet);
        assert!(doc.has_namespace);
        assert_eq!(
            doc.locs,
            vec![
                "https://rosas.example/",
                "https://rosas.example/menu?a=1&b=2",
                "https://rosas.example/about"
            ]
        );
        assert!(sitemap_diagnostics("/sitemap.xml", &doc, true).is_empty());
    }

    #[test]
    fn parses_a_sitemap_index() {
        let doc = parse_sitemap(
            r#"<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><sitemap><loc>https://x/a.xml</loc></sitemap></sitemapindex>"#,
        )
        .unwrap();
        assert_eq!(doc.kind, SitemapKind::Index);
        assert_eq!(doc.locs, vec!["https://x/a.xml"]);
    }

    #[test]
    fn rejects_html_and_broken_xml() {
        assert!(parse_sitemap("<!doctype html><html><body>404</body></html>").is_err());
        let err = parse_sitemap("<urlset><url><loc>x</loc></urlset>").unwrap_err();
        assert!(err.contains("XML error"), "{err}");
        assert!(parse_sitemap("").is_err());
    }

    #[test]
    fn sitemap_problems() {
        let doc = parse_sitemap(
            "<urlset><url><loc>/relative</loc></url><url><lastmod>2026-01-01</lastmod></url><url><loc>http://localhost:4321/x</loc></url></urlset>",
        )
        .unwrap();
        let d = sitemap_diagnostics("/sitemap.xml", &doc, false);
        let codes: Vec<&str> = d.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(
            codes,
            vec![
                "AUDIT_SITEMAP_NAMESPACE",
                "AUDIT_SITEMAP_NO_LOC",
                "AUDIT_SITEMAP_RELATIVE_URL",
                "AUDIT_SITEMAP_LOCALHOST"
            ]
        );
        // On a local audit the localhost URLs are expected.
        let d = sitemap_diagnostics("/sitemap.xml", &doc, true);
        assert!(!d.iter().any(|d| d.code == "AUDIT_SITEMAP_LOCALHOST"));
    }

    #[test]
    fn empty_urlset_is_an_error() {
        let doc = parse_sitemap(r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"/>"#)
            .unwrap();
        let d = sitemap_diagnostics("/sitemap.xml", &doc, true);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "AUDIT_SITEMAP_EMPTY");
    }
}
