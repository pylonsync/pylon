use crate::PluginError;

/// CSRF protection plugin.
///
/// Validates the `Origin` or `Referer` header on state-changing requests
/// (POST, PATCH, DELETE, PUT) against a list of allowed origins. This is
/// complementary to CORS: CORS controls which origins can *read* responses,
/// while CSRF protection ensures that state-changing requests originate from
/// trusted sources.
pub struct CsrfPlugin {
    allowed_origins: Vec<String>,
}

impl CsrfPlugin {
    /// Create a CSRF plugin with explicit allowed origins.
    pub fn new(allowed_origins: Vec<String>) -> Self {
        Self { allowed_origins }
    }

    /// Convenience constructor for local development. Allows both `localhost`
    /// and `127.0.0.1` on the given port.
    pub fn with_localhost(port: u16) -> Self {
        Self::new(vec![
            format!("http://localhost:{port}"),
            format!("http://127.0.0.1:{port}"),
        ])
    }

    /// Safe (read-only) methods that do not require origin validation.
    fn is_safe_method(method: &str) -> bool {
        matches!(method, "GET" | "HEAD" | "OPTIONS")
    }

    /// Check whether `origin` is in the allowlist. A wildcard entry (`"*"`)
    /// matches every origin.
    fn is_allowed_origin(&self, origin: &str) -> bool {
        self.allowed_origins
            .iter()
            .any(|o| o == origin || o == "*")
    }

    /// Extract the origin portion (`scheme://host[:port]`) from a full URL
    /// such as a `Referer` header value.
    ///
    /// ```text
    /// "http://example.com/path?q=1" -> Some("http://example.com")
    /// "https://a.b:8080/x"          -> Some("https://a.b:8080")
    /// "garbage"                      -> None
    /// ```
    fn origin_from_referer(referer: &str) -> Option<String> {
        // Split on '/' keeping at most 4 parts:
        //   "http:" "" "example.com" "path..."
        let parts: Vec<&str> = referer.splitn(4, '/').collect();
        if parts.len() >= 3 && !parts[2].is_empty() {
            Some(format!("{}//{}", parts[0], parts[2]))
        } else {
            None
        }
    }

    /// Validate an incoming request.
    ///
    /// For safe methods this always succeeds. For state-changing methods the
    /// `Origin` header is checked first; if absent the origin is derived from
    /// the `Referer` header. If neither header provides a trusted origin the
    /// request is rejected.
    pub fn check(
        &self,
        method: &str,
        origin: Option<&str>,
        referer: Option<&str>,
    ) -> Result<(), PluginError> {
        if Self::is_safe_method(method) {
            return Ok(());
        }

        let effective_origin = origin
            .map(String::from)
            .or_else(|| referer.and_then(Self::origin_from_referer));

        match effective_origin {
            Some(ref o) if self.is_allowed_origin(o) => Ok(()),
            Some(ref o) => Err(PluginError {
                code: "CSRF_REJECTED".into(),
                message: format!("Origin '{}' not allowed", o),
                status: 403,
            }),
            None => Err(PluginError {
                code: "CSRF_NO_ORIGIN".into(),
                message: "Missing Origin header on state-changing request".into(),
                status: 403,
            }),
        }
    }
}

impl crate::Plugin for CsrfPlugin {
    fn name(&self) -> &str {
        "csrf"
    }

    fn on_request(
        &self,
        _method: &str,
        _path: &str,
        _auth: &statecraft_auth::AuthContext,
    ) -> Result<(), PluginError> {
        // The Plugin trait's on_request does not receive HTTP headers, so CSRF
        // validation cannot happen here automatically. Use `check()` at the
        // HTTP layer where headers are available.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn localhost_plugin() -> CsrfPlugin {
        CsrfPlugin::with_localhost(3000)
    }

    // -- Safe methods always pass --

    #[test]
    fn safe_methods_pass_without_origin() {
        let csrf = localhost_plugin();
        for method in &["GET", "HEAD", "OPTIONS"] {
            assert!(csrf.check(method, None, None).is_ok());
        }
    }

    #[test]
    fn safe_methods_pass_with_bad_origin() {
        let csrf = localhost_plugin();
        assert!(csrf.check("GET", Some("https://evil.com"), None).is_ok());
    }

    // -- Matching origin passes --

    #[test]
    fn matching_origin_passes() {
        let csrf = localhost_plugin();
        assert!(csrf
            .check("POST", Some("http://localhost:3000"), None)
            .is_ok());
        assert!(csrf
            .check("DELETE", Some("http://127.0.0.1:3000"), None)
            .is_ok());
    }

    // -- Wrong origin rejected --

    #[test]
    fn wrong_origin_rejected() {
        let csrf = localhost_plugin();
        let err = csrf
            .check("POST", Some("https://evil.com"), None)
            .unwrap_err();
        assert_eq!(err.code, "CSRF_REJECTED");
        assert_eq!(err.status, 403);
    }

    // -- Missing origin rejected --

    #[test]
    fn missing_origin_rejected() {
        let csrf = localhost_plugin();
        let err = csrf.check("PUT", None, None).unwrap_err();
        assert_eq!(err.code, "CSRF_NO_ORIGIN");
        assert_eq!(err.status, 403);
    }

    // -- Wildcard allows all --

    #[test]
    fn wildcard_allows_all() {
        let csrf = CsrfPlugin::new(vec!["*".into()]);
        assert!(csrf
            .check("POST", Some("https://anything.example.com"), None)
            .is_ok());
        assert!(csrf
            .check("DELETE", Some("http://evil.com"), None)
            .is_ok());
    }

    // -- Referer extraction --

    #[test]
    fn origin_from_referer_extraction() {
        assert_eq!(
            CsrfPlugin::origin_from_referer("http://example.com/path?q=1"),
            Some("http://example.com".into())
        );
        assert_eq!(
            CsrfPlugin::origin_from_referer("https://a.b:8080/x/y"),
            Some("https://a.b:8080".into())
        );
        assert_eq!(CsrfPlugin::origin_from_referer("garbage"), None);
        assert_eq!(CsrfPlugin::origin_from_referer(""), None);
    }

    // -- Referer fallback when Origin is missing --

    #[test]
    fn referer_fallback_when_origin_missing() {
        let csrf = localhost_plugin();
        assert!(csrf
            .check("POST", None, Some("http://localhost:3000/some/path"))
            .is_ok());
    }

    #[test]
    fn referer_fallback_wrong_origin() {
        let csrf = localhost_plugin();
        let err = csrf
            .check("POST", None, Some("https://evil.com/attack"))
            .unwrap_err();
        assert_eq!(err.code, "CSRF_REJECTED");
    }

    // -- All state-changing methods are checked --

    #[test]
    fn all_state_changing_methods_require_origin() {
        let csrf = localhost_plugin();
        for method in &["POST", "PUT", "PATCH", "DELETE"] {
            assert!(csrf.check(method, None, None).is_err());
        }
    }

    // -- Plugin trait --

    #[test]
    fn plugin_name() {
        let csrf = localhost_plugin();
        assert_eq!(crate::Plugin::name(&csrf), "csrf");
    }
}
