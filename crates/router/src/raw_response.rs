//! Raw HTTP responses returned by webhook actions.
//!
//! An action invoked through `/api/webhooks/<name>` can return
//! `ctx.response({ status, headers, body })` (packages/functions). The
//! helper produces a marked object:
//!
//! ```json
//! { "__pylonResponse": 1, "status": 200,
//!   "headers": { "content-type": "text/xml" }, "body": "<Response/>" }
//! ```
//!
//! The webhook route sends that status, content type, headers, and body as
//! the HTTP response instead of JSON-encoding the value. Every other route
//! returns the marked object as ordinary JSON data.
//!
//! The server computes framing and connection headers, CORS headers, and its
//! security headers itself, so an action may not set them. `Set-Cookie` is
//! refused too: a webhook caller is a server, and a public webhook endpoint
//! must not be able to write cookies on the app's domain.
//!
//! The TypeScript helper applies the same rules so the error surfaces where
//! the handler builds the response. This module is the enforcement point,
//! because a handler can return a hand-built marked object. The two lists
//! MUST stay in sync (`packages/functions/src/response.ts`).

/// The marker key. Its value must be the number 1.
pub const MARKER: &str = "__pylonResponse";

/// Content type sent when the response names none.
pub const DEFAULT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

/// Header names the server sets or computes itself. Compared lowercase.
const RESERVED_HEADERS: &[&str] = &[
    // Framing and connection management (hop-by-hop).
    "connection",
    "content-length",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    // Session cookies belong to the auth routes.
    "set-cookie",
    // Security headers added to every response.
    "permissions-policy",
    "referrer-policy",
    "x-content-type-options",
    "x-frame-options",
    "x-xss-protection",
];

/// Header name prefixes the server owns (CORS).
const RESERVED_HEADER_PREFIXES: &[&str] = &["access-control-"];

/// A validated raw response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawResponse {
    pub status: u16,
    pub content_type: String,
    /// Extra headers, names lowercased, excluding `content-type`.
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// True when `value` carries the raw-response marker.
pub fn is_raw_response(value: &serde_json::Value) -> bool {
    value
        .get(MARKER)
        .and_then(|m| m.as_u64())
        .is_some_and(|m| m == 1)
}

/// Validate a marked value. `Err` carries a message for the operator log
/// and the error response.
pub fn parse(value: &serde_json::Value) -> Result<RawResponse, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "a raw response must be an object".to_string())?;

    let status = match obj.get("status") {
        None | Some(serde_json::Value::Null) => 200,
        Some(v) => v
            .as_u64()
            .filter(|s| (200..=599).contains(s))
            .ok_or_else(|| format!("status must be an integer from 200 to 599, got {v}"))?
            as u16,
    };

    let body = match obj.get("body") {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(_) => return Err("body must be a string".to_string()),
    };
    if !body.is_empty() && matches!(status, 204 | 205 | 304) {
        return Err(format!("status {status} cannot carry a body"));
    }

    let mut content_type: Option<String> = None;
    let mut headers = Vec::new();
    match obj.get("headers") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::Object(map)) => {
            for (name, value) in map {
                let value = value
                    .as_str()
                    .ok_or_else(|| format!("header \"{name}\" must have a string value"))?;
                let lower = check_header(name, value)?;
                if lower == "content-type" {
                    content_type = Some(value.to_string());
                } else {
                    headers.push((lower, value.to_string()));
                }
            }
        }
        Some(_) => return Err("headers must be an object of string values".to_string()),
    }

    Ok(RawResponse {
        status,
        content_type: content_type.unwrap_or_else(|| DEFAULT_CONTENT_TYPE.to_string()),
        headers,
        body,
    })
}

/// Check one header. Returns the lowercased name.
fn check_header(name: &str, value: &str) -> Result<String, String> {
    if name.is_empty() || !name.bytes().all(is_token_byte) {
        return Err(format!("header name {name:?} is not a valid HTTP token"));
    }
    // Visible ASCII, space, and tab only. Rejects CR/LF (header injection),
    // NUL, other control bytes, and non-ASCII bytes the HTTP servers cannot
    // encode.
    if !value
        .bytes()
        .all(|b| b == b'\t' || (0x20..0x7f).contains(&b))
    {
        return Err(format!(
            "header \"{name}\" has a value with control or non-ASCII characters"
        ));
    }
    let lower = name.to_ascii_lowercase();
    if RESERVED_HEADERS.contains(&lower.as_str())
        || RESERVED_HEADER_PREFIXES
            .iter()
            .any(|p| lower.starts_with(p))
    {
        return Err(format!("header \"{name}\" is set by the server"));
    }
    Ok(lower)
}

/// RFC 9110 `tchar`.
fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn marker_requires_the_number_one() {
        assert!(is_raw_response(&json!({ "__pylonResponse": 1 })));
        assert!(!is_raw_response(&json!({ "__pylonResponse": true })));
        assert!(!is_raw_response(&json!({ "__pylonResponse": "1" })));
        assert!(!is_raw_response(&json!({ "status": 200 })));
        assert!(!is_raw_response(&json!([1])));
    }

    #[test]
    fn parses_status_content_type_headers_and_body() {
        let r = parse(&json!({
            "__pylonResponse": 1,
            "status": 201,
            "headers": { "Content-Type": "text/xml", "X-Request-Id": "abc" },
            "body": "<Response/>",
        }))
        .unwrap();
        assert_eq!(r.status, 201);
        assert_eq!(r.content_type, "text/xml");
        assert_eq!(r.headers, vec![("x-request-id".into(), "abc".into())]);
        assert_eq!(r.body, "<Response/>");
    }

    #[test]
    fn defaults_to_empty_200_text_plain() {
        let r = parse(&json!({ "__pylonResponse": 1 })).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.content_type, DEFAULT_CONTENT_TYPE);
        assert!(r.headers.is_empty());
        assert_eq!(r.body, "");
    }

    #[test]
    fn rejects_out_of_range_or_non_integer_status() {
        for s in [
            json!(99),
            json!(101),
            json!(600),
            json!(200.5),
            json!("200"),
        ] {
            assert!(
                parse(&json!({ "__pylonResponse": 1, "status": s })).is_err(),
                "{s}"
            );
        }
    }

    #[test]
    fn rejects_body_on_no_content_statuses() {
        assert!(parse(&json!({ "__pylonResponse": 1, "status": 204, "body": "x" })).is_err());
        assert!(parse(&json!({ "__pylonResponse": 1, "status": 204 })).is_ok());
    }

    #[test]
    fn rejects_non_string_body() {
        assert!(parse(&json!({ "__pylonResponse": 1, "body": { "a": 1 } })).is_err());
    }

    #[test]
    fn rejects_header_injection() {
        for value in ["a\r\nSet-Cookie: x=1", "a\nb", "a\rb", "a\0b", "caf\u{e9}"] {
            let err = parse(&json!({
                "__pylonResponse": 1,
                "headers": { "X-Test": value },
            }))
            .unwrap_err();
            assert!(err.contains("X-Test"), "{err}");
        }
        for name in ["X Test", "X-Test\r\n", "", "X:Test", "X\u{e9}"] {
            assert!(
                parse(&json!({ "__pylonResponse": 1, "headers": { name: "v" } })).is_err(),
                "{name:?}"
            );
        }
    }

    #[test]
    fn rejects_server_owned_headers_in_any_case() {
        for name in [
            "Content-Length",
            "transfer-encoding",
            "Connection",
            "Set-Cookie",
            "Access-Control-Allow-Origin",
            "X-Frame-Options",
        ] {
            let err =
                parse(&json!({ "__pylonResponse": 1, "headers": { name: "v" } })).unwrap_err();
            assert!(err.contains("set by the server"), "{name}: {err}");
        }
    }

    #[test]
    fn allows_tab_in_values_and_cache_control() {
        let r = parse(&json!({
            "__pylonResponse": 1,
            "headers": { "Cache-Control": "max-age=0,\tmust-revalidate", "Location": "/next" },
            "status": 302,
        }))
        .unwrap();
        assert_eq!(r.headers.len(), 2);
    }
}
