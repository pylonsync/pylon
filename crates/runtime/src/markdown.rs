//! Markdown representations of SSR pages.
//!
//! Agents read pages; browsers read pages. They want different bytes for the
//! same URL. An agent handed 190KB of Tailwind-classed markup, inline JSON
//! hydration payloads, and SVG icon paths has to spend context stripping all
//! of it before it reaches the two paragraphs it came for. So Pylon serves the
//! SAME page as markdown when the client asks for markdown, two ways:
//!
//!   1. `Accept: text/markdown` on the page's own URL (the
//!      <https://acceptmarkdown.com> convention), and
//!   2. a `<path>.md` URL, for agents that fetch links without setting headers.
//!
//! HTML stays the default: a browser sending
//! `text/html,application/xhtml+xml,…` is unaffected, byte for byte.
//!
//! The conversion runs on the RENDERED HTML rather than on the React tree,
//! because the rendered HTML is the only stage that sees the final page —
//! layouts, metadata, and every `<Suspense>` boundary already resolved. It
//! reads the page's main landmark (`<main>`, then `[role=main]`, then
//! `<article>`, then `<body>`) so navigation and footer chrome never lands in
//! the output, and it parses with html5ever rather than scanning for tags,
//! because SSR output contains `<`-bearing text, inline JSON, and attribute
//! values that no regex survives.

use htmd::{HtmlToMarkdown, Node};
use markup5ever_rcdom::NodeData;
use std::rc::Rc;

/// A representation this server can produce for an SSR page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Representation {
    /// The rendered React page, as-is.
    Html,
    /// The page converted to markdown, as `text/markdown`.
    Markdown,
    /// The same markdown body, labelled `text/plain` — for a client that asked
    /// for plain text. Markdown IS plain text, and answering with a type the
    /// client did not list would defeat the point of negotiating.
    MarkdownAsText,
}

impl Representation {
    /// The `Content-Type` to send for this representation.
    pub fn content_type(self) -> &'static str {
        match self {
            Representation::Html => "text/html; charset=utf-8",
            Representation::Markdown => "text/markdown; charset=utf-8",
            Representation::MarkdownAsText => "text/plain; charset=utf-8",
        }
    }

    /// Does this representation need the HTML→markdown conversion?
    pub fn is_markdown(self) -> bool {
        matches!(
            self,
            Representation::Markdown | Representation::MarkdownAsText
        )
    }
}

/// The outcome of `Accept` negotiation for an SSR page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Negotiation {
    /// Serve this representation.
    Serve(Representation),
    /// The client ruled out everything this server can produce → 406.
    NotAcceptable,
}

/// One parsed media range from an `Accept` header.
#[derive(Debug, Clone, Copy)]
struct MediaRange<'a> {
    kind: &'a str,
    subtype: &'a str,
    /// Quality value, 0.0–1.0. Absent `q=` means 1.0 (RFC 9110 §12.4.2).
    q: f32,
    /// Position in the header, used only as the last tie-break.
    index: usize,
}

impl MediaRange<'_> {
    /// How specifically this range names a type: an exact `text/markdown` (2)
    /// beats `text/*` (1) beats `*/*` (0). RFC 9110 §12.5.1 ranks matches this
    /// way, and it is what keeps a browser's trailing `*/*` from outranking the
    /// `text/html` it listed first.
    fn specificity(&self) -> u8 {
        match (self.kind, self.subtype) {
            ("*", _) => 0,
            (_, "*") => 1,
            _ => 2,
        }
    }

    fn matches(&self, kind: &str, subtype: &str) -> bool {
        (self.kind == "*" || self.kind.eq_ignore_ascii_case(kind))
            && (self.subtype == "*" || self.subtype.eq_ignore_ascii_case(subtype))
    }
}

/// Parse an `Accept` header into media ranges. Malformed entries are dropped
/// rather than failing the request — a client with one bad range still gets a
/// page.
fn parse_accept(header: &str) -> Vec<MediaRange<'_>> {
    let mut out = Vec::new();
    for (index, part) in header.split(',').enumerate() {
        let mut params = part.split(';');
        let media = match params.next() {
            Some(m) => m.trim(),
            None => continue,
        };
        if media.is_empty() {
            continue;
        }
        let (kind, subtype) = match media.split_once('/') {
            Some((k, s)) => (k.trim(), s.trim()),
            // A bare token (`Accept: markdown`) is not a media range.
            None => continue,
        };
        if kind.is_empty() || subtype.is_empty() {
            continue;
        }
        // Only `q` matters here. Other parameters (`;charset=`, `;v=b3`) are
        // accepted and ignored — they never narrow what we can produce.
        let mut q = 1.0_f32;
        for param in params {
            let (name, value) = match param.split_once('=') {
                Some((n, v)) => (n.trim(), v.trim()),
                None => continue,
            };
            if name.eq_ignore_ascii_case("q") {
                q = value.parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0);
            }
        }
        out.push(MediaRange {
            kind,
            subtype,
            q,
            index,
        });
    }
    out
}

/// The best `(q, specificity, index)` among the ranges that match this type.
/// `None` when nothing matches — which is NOT the same as `q=0` (an explicit
/// refusal); both end up excluded, but only via this one path.
fn best_match(ranges: &[MediaRange<'_>], kind: &str, subtype: &str) -> Option<(f32, u8, usize)> {
    ranges
        .iter()
        .filter(|r| r.matches(kind, subtype))
        .map(|r| (r.q, r.specificity(), r.index))
        // Highest q wins; then the more specific range; then the earlier one.
        .max_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
                .then(b.2.cmp(&a.2))
        })
}

/// Choose a representation for an SSR page from the request's `Accept`.
///
/// HTML is the default in every ambiguous case: no header, `*/*`, `text/*`, or
/// a tie. A markdown variant is chosen only when the client scores it strictly
/// above HTML — by a higher `q`, by naming it more specifically, or by listing
/// it first at equal `q` and equal specificity.
pub fn negotiate(accept: Option<&str>) -> Negotiation {
    let header = match accept {
        Some(h) if !h.trim().is_empty() => h,
        // No Accept means "anything" (RFC 9110 §12.5.1).
        _ => return Negotiation::Serve(Representation::Html),
    };
    let ranges = parse_accept(header);
    if ranges.is_empty() {
        return Negotiation::Serve(Representation::Html);
    }

    let html = best_match(&ranges, "text", "html");
    // `text/x-markdown` is the pre-RFC-7763 spelling; still emitted by some
    // clients, and it means exactly the same thing.
    let markdown = [
        best_match(&ranges, "text", "markdown"),
        best_match(&ranges, "text", "x-markdown"),
    ]
    .into_iter()
    .flatten()
    .max_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
            .then(b.2.cmp(&a.2))
    });
    let text = best_match(&ranges, "text", "plain");

    // (score, representation) in DEFAULT-preference order: a tie keeps HTML.
    let mut candidates: Vec<((f32, u8, usize), Representation)> = Vec::new();
    if let Some(s) = html {
        candidates.push((s, Representation::Html));
    }
    if let Some(s) = markdown {
        candidates.push((s, Representation::Markdown));
    }
    if let Some(s) = text {
        candidates.push((s, Representation::MarkdownAsText));
    }
    // `q=0` is an explicit refusal, not a weak preference.
    candidates.retain(|((q, _, _), _)| *q > 0.0);
    if candidates.is_empty() {
        return Negotiation::NotAcceptable;
    }
    let mut best = candidates[0];
    for cand in candidates.into_iter().skip(1) {
        let ((q, spec, index), _) = cand;
        let ((bq, bspec, bindex), _) = best;
        let better = q > bq
            || (q == bq && spec > bspec)
            || (q == bq && spec == bspec && index < bindex);
        if better {
            best = cand;
        }
    }
    Negotiation::Serve(best.1)
}

/// Strip a `.md` suffix from a URL path, returning the page path it names.
///
/// `/about.md` → `/about`, `/product/sync.md` → `/product/sync`, and
/// `/index.md` → `/` (the home page, which has no basename of its own).
/// Returns `None` when the path does not name a markdown variant.
pub fn strip_md_suffix(path: &str) -> Option<String> {
    let rest = path.strip_suffix(".md")?;
    // `/.md` and `.md` name no page.
    if rest.is_empty() || rest.ends_with('/') {
        return None;
    }
    if rest == "/index" {
        return Some("/".to_string());
    }
    Some(rest.to_string())
}

/// The `.md` URL for a page path — the inverse of [`strip_md_suffix`], used for
/// the `Link: rel="alternate"` header on the HTML response.
pub fn md_url_for(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/index.md".to_string();
    }
    format!("{trimmed}.md")
}

/// Tags whose content is never page content. `nav`/`footer` are dropped
/// wherever they appear, so the `<body>` fallback (a page with no `<main>`)
/// still produces content rather than a menu dump.
const SKIP_TAGS: &[&str] = &[
    "script", "style", "noscript", "svg", "template", "iframe", "canvas", "nav", "footer", "head",
];

/// Page metadata lifted out of `<head>` for the markdown frontmatter.
#[derive(Debug, Default, PartialEq, Eq)]
struct HeadMeta {
    title: Option<String>,
    description: Option<String>,
    canonical: Option<String>,
}

/// Convert a rendered SSR page to markdown.
///
/// `url` is the absolute URL the page was requested at, recorded in the
/// frontmatter so an agent that stored the markdown can cite it.
pub fn html_to_markdown(html: &str, url: Option<&str>) -> String {
    let converter = HtmlToMarkdown::builder()
        .skip_tags(SKIP_TAGS.to_vec())
        // `<noscript>` content is markup, not text: parse it as DOM so the skip
        // above actually drops it instead of dumping raw tags into the body.
        .scripting_enabled(false)
        .build();
    let tree = match converter.html_to_tree(html) {
        Ok(t) => t,
        // A parse failure means html5ever could not build a document at all.
        // Nothing useful to say in markdown; the caller falls back to HTML.
        Err(_) => return String::new(),
    };
    let meta = extract_head_meta(&tree);
    let root = content_root(&tree).unwrap_or_else(|| Rc::clone(&tree));
    let body = converter.tree_to_markdown(&root);

    let mut out = String::new();
    let frontmatter = build_frontmatter(&meta, url);
    if !frontmatter.is_empty() {
        out.push_str(&frontmatter);
        out.push('\n');
    }
    out.push_str(body.trim());
    out.push('\n');
    out
}

/// YAML frontmatter carrying the page's identity. Omitted entirely when the
/// page has no title, no description, and no URL to record — an empty
/// `---\n---` block is noise.
fn build_frontmatter(meta: &HeadMeta, url: Option<&str>) -> String {
    let canonical = meta.canonical.as_deref().or(url);
    let mut lines: Vec<String> = Vec::new();
    if let Some(t) = meta.title.as_deref() {
        lines.push(format!("title: {}", yaml_quote(t)));
    }
    if let Some(d) = meta.description.as_deref() {
        lines.push(format!("description: {}", yaml_quote(d)));
    }
    if let Some(u) = canonical {
        lines.push(format!("url: {}", yaml_quote(u)));
    }
    if lines.is_empty() {
        return String::new();
    }
    format!("---\n{}\n---\n", lines.join("\n"))
}

/// Double-quoted YAML scalar. Quoting unconditionally avoids the whole class of
/// "a colon in the title broke the document" bugs.
fn yaml_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The element whose subtree holds the page's content: `<main>`, then
/// `[role=main]`, then `<article>`, then `<body>`.
fn content_root(tree: &Rc<Node>) -> Option<Rc<Node>> {
    if let Some(n) = find_element(tree, &|name, attrs| {
        name == "main"
            || attrs
                .iter()
                .any(|(k, v)| k == "role" && v.eq_ignore_ascii_case("main"))
    }) {
        return Some(n);
    }
    if let Some(n) = find_element(tree, &|name, _| name == "article") {
        return Some(n);
    }
    find_element(tree, &|name, _| name == "body")
}

/// Depth-first search for the first element matching `pred`, which receives the
/// lower-cased tag name and the element's attributes.
fn find_element(
    node: &Rc<Node>,
    pred: &dyn Fn(&str, &[(String, String)]) -> bool,
) -> Option<Rc<Node>> {
    if let NodeData::Element {
        ref name,
        ref attrs,
        ..
    } = node.data
    {
        let tag = name.local.as_ref().to_ascii_lowercase();
        let attr_pairs: Vec<(String, String)> = attrs
            .borrow()
            .iter()
            .map(|a| {
                (
                    a.name.local.as_ref().to_ascii_lowercase(),
                    a.value.to_string(),
                )
            })
            .collect();
        if pred(&tag, &attr_pairs) {
            return Some(Rc::clone(node));
        }
    }
    for child in node.children.borrow().iter() {
        if let Some(found) = find_element(child, pred) {
            return Some(found);
        }
    }
    None
}

/// Read `<title>`, `<meta name="description">`, and `<link rel="canonical">`.
fn extract_head_meta(tree: &Rc<Node>) -> HeadMeta {
    let mut meta = HeadMeta::default();
    walk_head(tree, &mut meta);
    meta
}

fn walk_head(node: &Rc<Node>, meta: &mut HeadMeta) {
    if let NodeData::Element {
        ref name,
        ref attrs,
        ..
    } = node.data
    {
        let tag = name.local.as_ref().to_ascii_lowercase();
        let attr = |wanted: &str| -> Option<String> {
            attrs.borrow().iter().find_map(|a| {
                if a.name.local.as_ref().eq_ignore_ascii_case(wanted) {
                    Some(a.value.to_string())
                } else {
                    None
                }
            })
        };
        match tag.as_str() {
            // The FIRST title wins, matching how browsers and crawlers read a
            // document with more than one (Pylon's layout + page can each emit
            // one — see the ROUTE_OWNS_TITLE dance in app layouts).
            "title" if meta.title.is_none() => {
                let text = text_content(node);
                if !text.trim().is_empty() {
                    meta.title = Some(text.trim().to_string());
                }
            }
            "meta" if meta.description.is_none() => {
                if attr("name").is_some_and(|n| n.eq_ignore_ascii_case("description")) {
                    if let Some(content) = attr("content") {
                        if !content.trim().is_empty() {
                            meta.description = Some(content.trim().to_string());
                        }
                    }
                }
            }
            "link" if meta.canonical.is_none() => {
                if attr("rel").is_some_and(|r| r.eq_ignore_ascii_case("canonical")) {
                    if let Some(href) = attr("href") {
                        if !href.trim().is_empty() {
                            meta.canonical = Some(href.trim().to_string());
                        }
                    }
                }
            }
            // The body can't contain head metadata; stop descending.
            "body" => return,
            _ => {}
        }
    }
    for child in node.children.borrow().iter() {
        walk_head(child, meta);
    }
}

/// Concatenated text of a node's subtree.
fn text_content(node: &Rc<Node>) -> String {
    let mut out = String::new();
    collect_text(node, &mut out);
    out
}

fn collect_text(node: &Rc<Node>, out: &mut String) {
    if let NodeData::Text { ref contents } = node.data {
        out.push_str(&contents.borrow());
    }
    for child in node.children.borrow().iter() {
        collect_text(child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn served(accept: &str) -> Representation {
        match negotiate(Some(accept)) {
            Negotiation::Serve(r) => r,
            Negotiation::NotAcceptable => panic!("expected a representation for {accept:?}"),
        }
    }

    #[test]
    fn html_is_the_default() {
        assert_eq!(negotiate(None), Negotiation::Serve(Representation::Html));
        assert_eq!(negotiate(Some("")), Negotiation::Serve(Representation::Html));
        // curl's default, and every crawler that doesn't care.
        assert_eq!(served("*/*"), Representation::Html);
        assert_eq!(served("text/*"), Representation::Html);
    }

    #[test]
    fn a_browser_gets_html() {
        assert_eq!(
            served("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,*/*;q=0.8"),
            Representation::Html
        );
    }

    #[test]
    fn an_explicit_markdown_request_gets_markdown() {
        assert_eq!(served("text/markdown"), Representation::Markdown);
        assert_eq!(served("text/x-markdown"), Representation::Markdown);
        // More specific than the wildcard it rides with.
        assert_eq!(served("text/markdown, */*"), Representation::Markdown);
        assert_eq!(served("text/markdown;q=1.0, text/html;q=0.9"), Representation::Markdown);
    }

    #[test]
    fn q_values_decide_before_specificity() {
        assert_eq!(served("text/markdown;q=0.5, text/html"), Representation::Html);
        assert_eq!(served("text/html;q=0.1, text/markdown;q=0.2"), Representation::Markdown);
    }

    #[test]
    fn header_order_breaks_an_exact_tie() {
        // Same q, same specificity: honor what the client listed first.
        assert_eq!(served("text/markdown, text/html"), Representation::Markdown);
        assert_eq!(served("text/html, text/markdown"), Representation::Html);
    }

    #[test]
    fn q_zero_is_a_refusal() {
        assert_eq!(served("text/html;q=0, text/markdown"), Representation::Markdown);
        assert_eq!(
            negotiate(Some("text/html;q=0, text/markdown;q=0, text/plain;q=0")),
            Negotiation::NotAcceptable
        );
        assert_eq!(negotiate(Some("image/png")), Negotiation::NotAcceptable);
        assert_eq!(negotiate(Some("application/json")), Negotiation::NotAcceptable);
    }

    #[test]
    fn plain_text_gets_markdown_under_its_own_type() {
        let r = served("text/plain");
        assert_eq!(r, Representation::MarkdownAsText);
        assert!(r.is_markdown());
        assert_eq!(r.content_type(), "text/plain; charset=utf-8");
    }

    #[test]
    fn malformed_ranges_are_ignored_not_fatal() {
        assert_eq!(served("garbage,, text/markdown"), Representation::Markdown);
        assert_eq!(served("text/markdown;q=notanumber"), Representation::Markdown);
    }

    #[test]
    fn md_suffix_maps_to_its_page() {
        assert_eq!(strip_md_suffix("/about.md").as_deref(), Some("/about"));
        assert_eq!(
            strip_md_suffix("/product/sync.md").as_deref(),
            Some("/product/sync")
        );
        assert_eq!(strip_md_suffix("/index.md").as_deref(), Some("/"));
        assert_eq!(strip_md_suffix("/about"), None);
        assert_eq!(strip_md_suffix("/.md"), None);
        assert_eq!(strip_md_suffix("/docs/.md"), None);
    }

    #[test]
    fn md_url_is_the_inverse() {
        assert_eq!(md_url_for("/"), "/index.md");
        assert_eq!(md_url_for("/about"), "/about.md");
        assert_eq!(md_url_for("/product/sync/"), "/product/sync.md");
        for path in ["/", "/about", "/product/sync"] {
            let md = md_url_for(path);
            assert_eq!(strip_md_suffix(&md).as_deref(), Some(path));
        }
    }

    #[test]
    fn converts_the_main_landmark_and_drops_chrome() {
        let html = r#"<!doctype html><html><head><title>Docs — Pylon</title>
            <meta name="description" content="A framework."></head>
            <body><nav><a href="/x">Nav link</a></nav>
            <main><h1>Entities</h1><p>Typed tables, <a href="/policies">policies</a>.</p>
            <ul><li>one</li><li>two</li></ul></main>
            <footer><a href="/legal">Terms</a></footer>
            <script>window.__DATA__ = {"a":1}</script></body></html>"#;
        let md = html_to_markdown(html, Some("https://example.com/docs"));
        assert!(md.starts_with("---\ntitle: \"Docs — Pylon\""), "{md}");
        assert!(md.contains("description: \"A framework.\""), "{md}");
        assert!(md.contains("url: \"https://example.com/docs\""), "{md}");
        assert!(md.contains("# Entities"), "{md}");
        assert!(md.contains("[policies](/policies)"), "{md}");
        assert!(md.contains("- one"), "{md}");
        assert!(!md.contains("Nav link"), "nav leaked: {md}");
        assert!(!md.contains("Terms"), "footer leaked: {md}");
        assert!(!md.contains("__DATA__"), "script leaked: {md}");
    }

    #[test]
    fn falls_back_to_body_when_there_is_no_main() {
        let html = "<!doctype html><html><head><title>T</title></head><body>\
            <nav><a href=\"/n\">Menu</a></nav><header><h1>Hero</h1></header>\
            <section><p>Body copy.</p></section><footer>Legal</footer></body></html>";
        let md = html_to_markdown(html, None);
        assert!(md.contains("# Hero"), "{md}");
        assert!(md.contains("Body copy."), "{md}");
        assert!(!md.contains("Menu"), "nav leaked: {md}");
        assert!(!md.contains("Legal"), "footer leaked: {md}");
    }

    #[test]
    fn prefers_a_canonical_link_over_the_request_url() {
        let html = "<html><head><title>T</title>\
            <link rel=\"canonical\" href=\"https://www.example.com/a\"></head>\
            <body><main><p>x</p></main></body></html>";
        let md = html_to_markdown(html, Some("https://example.com/a?utm=1"));
        assert!(md.contains("url: \"https://www.example.com/a\""), "{md}");
        assert!(!md.contains("utm=1"), "{md}");
    }

    #[test]
    fn a_title_with_a_colon_stays_valid_yaml() {
        let html = "<html><head><title>Pylon: the framework</title></head>\
            <body><main><p>x</p></main></body></html>";
        let md = html_to_markdown(html, None);
        assert!(md.contains("title: \"Pylon: the framework\""), "{md}");
    }

    #[test]
    fn role_main_counts_as_the_landmark() {
        let html = "<html><body><div role=\"main\"><h2>Scoped</h2></div>\
            <div><p>Outside</p></div></body></html>";
        let md = html_to_markdown(html, None);
        assert!(md.contains("## Scoped"), "{md}");
        assert!(!md.contains("Outside"), "{md}");
    }

    #[test]
    fn an_empty_document_does_not_panic() {
        assert_eq!(html_to_markdown("", None).trim(), "");
    }
}
