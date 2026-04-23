use crate::{Plugin, PluginError};
use pylon_auth::AuthContext;

/// CORS plugin. Validates request origins against an allowlist.
pub struct CorsPlugin {
    /// Allowed origins. Empty = allow all ("*").
    pub allowed_origins: Vec<String>,
    /// Whether to allow credentials.
    pub allow_credentials: bool,
}

impl CorsPlugin {
    /// Allow all origins.
    pub fn allow_all() -> Self {
        Self {
            allowed_origins: vec![],
            allow_credentials: false,
        }
    }

    /// Allow specific origins.
    pub fn new(origins: Vec<String>) -> Self {
        Self {
            allowed_origins: origins,
            allow_credentials: true,
        }
    }

    /// Check if an origin is allowed.
    pub fn is_allowed(&self, origin: &str) -> bool {
        if self.allowed_origins.is_empty() {
            return true; // wildcard
        }
        self.allowed_origins.iter().any(|o| o == origin || o == "*")
    }

    /// Get the Access-Control-Allow-Origin header value.
    pub fn allow_origin_header(&self, request_origin: Option<&str>) -> String {
        if self.allowed_origins.is_empty() {
            return "*".to_string();
        }
        match request_origin {
            Some(origin) if self.is_allowed(origin) => origin.to_string(),
            _ => String::new(),
        }
    }
}

impl Plugin for CorsPlugin {
    fn name(&self) -> &str {
        "cors"
    }

    fn on_request(
        &self,
        _method: &str,
        _path: &str,
        _auth: &AuthContext,
    ) -> Result<(), PluginError> {
        // CORS is handled at the HTTP layer (headers), not here.
        // This plugin provides the configuration; the server reads it.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_all() {
        let cors = CorsPlugin::allow_all();
        assert!(cors.is_allowed("http://localhost:3000"));
        assert!(cors.is_allowed("https://example.com"));
        assert_eq!(cors.allow_origin_header(Some("http://localhost:3000")), "*");
    }

    #[test]
    fn specific_origins() {
        let cors = CorsPlugin::new(vec![
            "http://localhost:3000".into(),
            "https://myapp.com".into(),
        ]);

        assert!(cors.is_allowed("http://localhost:3000"));
        assert!(cors.is_allowed("https://myapp.com"));
        assert!(!cors.is_allowed("https://evil.com"));
    }

    #[test]
    fn allow_origin_header_matches() {
        let cors = CorsPlugin::new(vec!["https://myapp.com".into()]);

        assert_eq!(
            cors.allow_origin_header(Some("https://myapp.com")),
            "https://myapp.com"
        );
        assert_eq!(cors.allow_origin_header(Some("https://evil.com")), "");
        assert_eq!(cors.allow_origin_header(None), "");
    }
}
