use pylon_kernel::AppManifest;

/// Bundled Studio HTML — produced by `crates/studio_api/web/` (Vite +
/// React + shadcn). The web/ project builds a single self-contained
/// HTML file via `vite-plugin-singlefile`; we embed it at compile time
/// and substitute three placeholders per request:
///
///   __PYLON_NAME__         — HTML-escaped app name (used in <title>)
///   __PYLON_API_BASE__     — JS-string-escaped API origin
///   __PYLON_MANIFEST_JSON__ — script-safe manifest JSON literal
///
/// To rebuild Studio: `cd crates/studio_api/web && bun run build`.
const STUDIO_HTML: &str = include_str!("../web/dist/index.html");

/// Generate the Studio inspector HTML.
///
/// XSS prevention. The manifest is developer-authored but contains
/// user-shaped strings (entity names, app name from package.json), so
/// every interpolated value is treated as untrusted:
///
///   - JSON embedded inside <script>: `</script` (case-insensitive) is
///     broken with a backslash so a crafted entity name can't close
///     the surrounding tag. U+2028 / U+2029 are escaped because they
///     terminate JS string literals in some parsers.
///   - Strings in HTML body (title): full HTML-encoded.
///   - Strings in JS string literals (api_base): backslash + Unicode
///     escapes, plus `<` → `\u003c` so a `</script` can't slip
///     through any bundler that re-quotes the value.
pub fn generate_studio_html(manifest: &AppManifest, api_base: &str) -> String {
    let manifest_json =
        escape_script_json(&serde_json::to_string(manifest).unwrap_or_else(|_| "{}".into()));
    let name = html_escape(&manifest.name);
    let api = js_string_escape(api_base);

    STUDIO_HTML
        .replace("__PYLON_NAME__", &name)
        .replace("__PYLON_API_BASE__", &api)
        // Replace LAST so the JSON's own braces / quotes can't be
        // re-interpreted by a subsequent .replace.
        .replace("__PYLON_MANIFEST_JSON__", &manifest_json)
}

/// Escape `</script` (case-insensitive) and the JS line separators
/// U+2028 / U+2029 inside JSON embedded in a `<script>` block.
///
/// HTML tag matching is ASCII-case-insensitive, so `</ScRiPt>` would
/// otherwise close the script tag — a one-shot XSS via a crafted
/// entity name. Insert a backslash between the `<` and the `/`, which
/// is still valid JSON (backslashes can escape any char) and no
/// longer looks like a closing tag to the HTML parser.
///
/// U+2028 and U+2029 are JS "line terminators" that close unclosed
/// string literals and break Babel's parser; escape them as
/// `\u00XX` sequences inside the JSON.
fn escape_script_json(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let sb = s.as_bytes();
    let lb = lower.as_bytes();
    let needle = b"</script";
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    while i < sb.len() {
        if i + needle.len() <= sb.len() && &lb[i..i + needle.len()] == needle {
            out.push('<');
            out.push('\\');
            out.push_str(&s[i + 1..i + needle.len()]);
            i += needle.len();
        } else {
            // Append one char (not one byte) to avoid corrupting
            // multi-byte UTF-8 sequences like U+2028.
            let c = s[i..].chars().next().expect("mid-string must yield a char");
            out.push(c);
            i += c.len_utf8();
        }
    }
    out.replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn js_string_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            // Escape '<' so a stray "</script" inside an api_base
            // can't close the surrounding script tag if a future
            // bundler re-quotes the string.
            '<' => out.push_str("\\u003c"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest() -> AppManifest {
        // Shape mirrors the example app — gives the security tests a
        // realistic embedded payload. We patch in a known query name
        // so includes_manifest can assert it survived the build.
        let mut m: AppManifest = serde_json::from_str(include_str!(
            "../../../examples/todo-app/pylon.manifest.json"
        ))
        .unwrap();
        m.queries.push(pylon_kernel::ManifestQuery {
            name: "todosByAuthor".into(),
            input: vec![],
        });
        m
    }

    #[test]
    fn generates_html() {
        let html = generate_studio_html(&test_manifest(), "http://localhost:4321");
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("Pylon Studio")); // <title> base
        assert!(html.contains("todo-app")); // manifest name made it in
    }

    #[test]
    fn html_escapes_manifest_name_in_title() {
        // The <title> tag interpolates manifest.name as raw HTML. An
        // attacker-influenced name must not break out of the title element.
        let mut m = test_manifest();
        m.name = "</title><script>alert('x')</script>".into();
        let html = generate_studio_html(&m, "http://localhost:4321");

        let start = html.find("<title>").expect("no <title>");
        let end = html[start..].find("</title>").expect("no </title>");
        let title = &html[start..start + end];
        assert!(
            !title.contains("<script>"),
            "XSS: raw <script> tag inside <title>: {title:?}"
        );
        assert!(title.contains("&lt;/title&gt;"));
    }

    #[test]
    fn api_base_is_js_escaped() {
        // api_base is interpolated into `window.__PYLON_API__ = "…";`
        // as a JS string literal. A quote in the input must appear
        // backslash-escaped (or Unicode-escaped) in the output so it
        // can't close the string early.
        let m = test_manifest();
        let html = generate_studio_html(&m, "http://example.com\"; alert(1); //");
        let needle = "window.__PYLON_API__ = \"";
        let start = html.find(needle).expect("no PYLON_API assignment");
        let rest = &html[start + needle.len()..];
        let mut idx = 0;
        let bytes = rest.as_bytes();
        while idx < bytes.len() {
            if bytes[idx] == b'"' {
                let mut backslashes = 0usize;
                let mut j = idx;
                while j > 0 && bytes[j - 1] == b'\\' {
                    backslashes += 1;
                    j -= 1;
                }
                if backslashes % 2 == 0 {
                    break;
                }
            }
            idx += 1;
        }
        let literal = &rest[..idx];
        assert!(
            !literal.contains("; alert(1); //")
                || literal.contains("\\\"; alert(1); //")
                || literal.contains("\\u0022"),
            "XSS: raw quote broke out of JS string: {literal:?}"
        );
    }

    #[test]
    fn escape_script_json_is_case_insensitive() {
        // Mixed-case </ScRiPt> would otherwise close the <script>
        // tag that surrounds the embedded MANIFEST JSON. Any casing
        // must get the backslash break.
        let mut m = test_manifest();
        m.entities[0].name = "E</ScRiPt><svg onload=alert(1)>".into();
        let html = generate_studio_html(&m, "http://ok");
        let embedded = find_manifest_json_block(&html);
        let lower = embedded.to_ascii_lowercase();
        let mut pos = 0;
        while let Some(idx) = lower[pos..].find("</script") {
            let abs = pos + idx;
            let preceded_by_backslash = abs > 0 && embedded.as_bytes()[abs - 1] == b'\\';
            assert!(
                preceded_by_backslash,
                "unescaped </script at byte {abs} in: {embedded}"
            );
            pos = abs + 1;
        }
    }

    #[test]
    fn escape_script_json_handles_line_separator() {
        // U+2028 / U+2029 close JS string literals in some parsers.
        // Escape them so a crafted name can't break the inline JSON.
        let mut m = test_manifest();
        m.entities[0].name = "ok\u{2028}oops".into();
        let html = generate_studio_html(&m, "http://ok");
        let embedded = find_manifest_json_block(&html);
        assert!(
            !embedded.contains('\u{2028}'),
            "U+2028 leaked into the embedded manifest JSON"
        );
        assert!(embedded.contains("\\u2028"));
    }

    fn find_manifest_json_block(html: &str) -> String {
        // The manifest is injected as `window.__PYLON_MANIFEST__ = {...};`
        let needle = "window.__PYLON_MANIFEST__ = ";
        let start = html.find(needle).expect("no PYLON_MANIFEST assignment");
        let after = &html[start..];
        // Find the terminating `;` followed by either newline, space,
        // or end-of-script — generous because the bundler may strip
        // whitespace.
        let end = after
            .find("</script")
            .or_else(|| after.find("\n"))
            .unwrap_or(after.len().min(200_000));
        after[..end].to_string()
    }

    #[test]
    fn escape_helpers_directly() {
        let mut m = test_manifest();
        m.name = "A&B <C>".into();
        let html = generate_studio_html(&m, "http://ok");
        let start = html.find("<title>").unwrap();
        let end = html[start..].find("</title>").unwrap();
        assert!(html[start..start + end].contains("A&amp;B &lt;C&gt;"));
    }

    #[test]
    fn includes_entity_data() {
        let html = generate_studio_html(&test_manifest(), "http://localhost:4321");
        // Entities appear inside the embedded MANIFEST JSON.
        assert!(html.contains("\"User\""));
        assert!(html.contains("\"Todo\""));
    }

    #[test]
    fn includes_manifest() {
        let html = generate_studio_html(&test_manifest(), "http://localhost:4321");
        assert!(html.contains("manifest_version"));
        assert!(html.contains("todosByAuthor"));
    }
}
