//! The public URL of a request, as the client addressed it.
//!
//! A reverse proxy (Fly, Cloudflare, nginx) sits between the client and
//! Pylon, so the socket never sees the client's scheme, and the `Host`
//! header is whatever the last hop sent. Webhook providers that sign the
//! full URL (Twilio's `X-Twilio-Signature`) need the URL the provider
//! called, so this module rebuilds it under the same host-trust rules the
//! SSR origin resolver uses (`resolveOrigin` in
//! packages/functions/src/ssr-runtime.ts, `ssr_cache_host_bucket` in the
//! runtime's frontend).
//!
//! A request host is trusted when it is loopback, the host of
//! `PYLON_PUBLIC_URL`, `PYLON_CANONICAL_HOST`, a `PYLON_TRUSTED_HOSTS`
//! entry, or a platform (tenant) domain of the app.

/// Operator configuration that decides which request hosts are trusted and
/// what the canonical origin is. Read from the environment with
/// [`PublicOriginConfig::from_env`]; tests build it directly.
#[derive(Debug, Clone, Default)]
pub struct PublicOriginConfig {
    /// `PYLON_PUBLIC_URL`, trailing slashes trimmed. Empty when unset.
    pub public_url: String,
    /// `PYLON_CANONICAL_HOST`. Empty when unset.
    pub canonical_host: String,
    /// `PYLON_TRUSTED_HOSTS`, split on commas.
    pub trusted_hosts: Vec<String>,
}

impl PublicOriginConfig {
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        Self {
            public_url: var("PYLON_PUBLIC_URL")
                .trim()
                .trim_end_matches('/')
                .to_string(),
            canonical_host: var("PYLON_CANONICAL_HOST").trim().to_string(),
            trusted_hosts: var("PYLON_TRUSTED_HOSTS")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }

    /// True when `host` (lowercase authority, as [`host_of`] returns) is in
    /// the operator allowlist: the `PYLON_PUBLIC_URL` host,
    /// `PYLON_CANONICAL_HOST`, or a `PYLON_TRUSTED_HOSTS` entry.
    pub fn allows_host(&self, host: &str) -> bool {
        if host.is_empty() {
            return false;
        }
        std::iter::once(self.public_url.as_str())
            .chain(std::iter::once(self.canonical_host.as_str()))
            .chain(self.trusted_hosts.iter().map(String::as_str))
            .map(host_of)
            .any(|h| !h.is_empty() && h == host)
    }

    /// True when `host` is loopback, in the operator allowlist, or accepted by
    /// `tenant_host` (the app's platform domains).
    pub fn trusts_host(&self, host: &str, tenant_host: impl Fn(&str) -> bool) -> bool {
        !host.is_empty() && (is_loopback_host(host) || self.allows_host(host) || tenant_host(host))
    }

    /// The canonical origin (`scheme://authority`): the origin of
    /// `PYLON_PUBLIC_URL`, else `https://` + `PYLON_CANONICAL_HOST`. `None`
    /// when neither is set.
    pub fn canonical_origin(&self) -> Option<String> {
        if let Some((scheme, rest)) = self.public_url.split_once("://") {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            if !authority.is_empty() {
                return Some(format!(
                    "{}://{}",
                    scheme.to_ascii_lowercase(),
                    authority.to_ascii_lowercase()
                ));
            }
        }
        let canon = host_of(&self.canonical_host);
        (!canon.is_empty()).then(|| format!("https://{canon}"))
    }
}

/// The lowercase authority (`host[:port]`) of a URL or bare host value.
/// `https://App.example.com:8443/base` → `app.example.com:8443`.
pub fn host_of(value: &str) -> String {
    let t = value.trim();
    if t.is_empty() {
        return String::new();
    }
    let host = match t.find("://") {
        Some(i) => t[i + 3..].split('/').next().unwrap_or(""),
        None => t.trim_matches('/'),
    };
    host.to_ascii_lowercase()
}

/// Loopback host? Mirrors `LOOPBACK_HOST` in ssr-runtime.ts:
/// `/^(localhost|127\.|\[?::1|0\.0\.0\.0)/`.
pub fn is_loopback_host(host: &str) -> bool {
    host.starts_with("localhost")
        || host.starts_with("127.")
        || host.starts_with("::1")
        || host.starts_with("[::1")
        || host.starts_with("0.0.0.0")
}

/// The full URL the client requested: `scheme://host` + `path_and_query`.
///
/// Host, in order of precedence:
/// 1. The first `X-Forwarded-Host` entry, when trusted.
/// 2. The `Host` header, when trusted.
/// 3. The canonical origin ([`PublicOriginConfig::canonical_origin`]). Its
///    scheme is used as is, and `X-Forwarded-Proto` is ignored.
/// 4. When no canonical origin is configured, the untrusted
///    `X-Forwarded-Host` or `Host` value. This covers local development
///    behind a tunnel. Set `PYLON_PUBLIC_URL` or `PYLON_TRUSTED_HOSTS` in
///    production.
///
/// Scheme for cases 1, 2, and 4: the first `X-Forwarded-Proto` entry when it
/// is `http` or `https`, else `http` for a loopback host and `https` for any
/// other host. The forwarded scheme only switches between the http and https
/// forms of the same trusted host, so a client that forges it can only make
/// a signature check fail.
///
/// `headers` are (name, value) pairs; names match case-insensitively.
/// `path_and_query` is the request target as received. With no usable host
/// at all, the return value is `path_and_query` alone.
pub fn request_public_url(
    headers: &[(String, String)],
    path_and_query: &str,
    cfg: &PublicOriginConfig,
    tenant_host: impl Fn(&str) -> bool,
) -> String {
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    let first_entry = |v: &str| host_of(v.split(',').next().unwrap_or(""));
    let forwarded_host = header("x-forwarded-host").map(first_entry);
    let host = header("host").map(first_entry);
    let candidates: Vec<String> = [forwarded_host, host]
        .into_iter()
        .flatten()
        .filter(|h| is_valid_authority(h))
        .collect();

    let forwarded_proto = header("x-forwarded-proto")
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| v == "http" || v == "https");
    let scheme_for = |host: &str| {
        forwarded_proto.clone().unwrap_or_else(|| {
            if is_loopback_host(host) {
                "http".to_string()
            } else {
                "https".to_string()
            }
        })
    };

    if let Some(h) = candidates.iter().find(|h| cfg.trusts_host(h, &tenant_host)) {
        return format!("{}://{h}{path_and_query}", scheme_for(h));
    }
    if let Some(origin) = cfg.canonical_origin() {
        return format!("{origin}{path_and_query}");
    }
    if let Some(h) = candidates.first() {
        return format!("{}://{h}{path_and_query}", scheme_for(h));
    }
    path_and_query.to_string()
}

/// A host header value safe to paste into a URL: non-empty, and only the
/// characters a `host[:port]` or `[ipv6]:port` authority can hold. Rejects
/// values carrying `/`, `@`, `?`, `#`, spaces, or control characters.
fn is_valid_authority(h: &str) -> bool {
    !h.is_empty()
        && h.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']' | b'_')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdrs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn cfg(public_url: &str, trusted: &[&str]) -> PublicOriginConfig {
        PublicOriginConfig {
            public_url: public_url.to_string(),
            canonical_host: String::new(),
            trusted_hosts: trusted.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn no_tenants(_: &str) -> bool {
        false
    }

    const PATH: &str = "/api/webhooks/sms?x=1&y=%2B1";

    #[test]
    fn trusted_host_uses_forwarded_proto_and_keeps_query() {
        let h = hdrs(&[("Host", "app.example.com"), ("X-Forwarded-Proto", "https")]);
        let url = request_public_url(&h, PATH, &cfg("https://app.example.com", &[]), no_tenants);
        assert_eq!(url, "https://app.example.com/api/webhooks/sms?x=1&y=%2B1");
    }

    #[test]
    fn trusted_forwarded_host_wins_over_host() {
        // A proxy that rewrites Host to the upstream name keeps the public
        // name in X-Forwarded-Host.
        let h = hdrs(&[
            ("host", "app-internal.fly.dev"),
            ("x-forwarded-host", "app.example.com"),
            ("x-forwarded-proto", "https"),
        ]);
        let url = request_public_url(&h, PATH, &cfg("", &["app.example.com"]), no_tenants);
        assert_eq!(url, "https://app.example.com/api/webhooks/sms?x=1&y=%2B1");
    }

    #[test]
    fn untrusted_forwarded_host_falls_back_to_trusted_host() {
        let h = hdrs(&[
            ("host", "app.example.com"),
            ("x-forwarded-host", "evil.example.net"),
        ]);
        let url = request_public_url(&h, PATH, &cfg("https://app.example.com", &[]), no_tenants);
        assert_eq!(url, "https://app.example.com/api/webhooks/sms?x=1&y=%2B1");
    }

    #[test]
    fn untrusted_host_uses_public_url_origin_and_ignores_forwarded_proto() {
        let h = hdrs(&[
            ("host", "evil.example.net"),
            ("x-forwarded-host", "evil.example.net"),
            ("x-forwarded-proto", "http"),
        ]);
        let url = request_public_url(
            &h,
            PATH,
            &cfg("https://App.Example.com:8443/base/", &[]),
            no_tenants,
        );
        assert_eq!(
            url,
            "https://app.example.com:8443/api/webhooks/sms?x=1&y=%2B1"
        );
    }

    #[test]
    fn untrusted_host_uses_canonical_host_when_no_public_url() {
        let h = hdrs(&[("host", "evil.example.net")]);
        let c = PublicOriginConfig {
            canonical_host: "www.example.com".into(),
            ..Default::default()
        };
        let url = request_public_url(&h, "/p", &c, no_tenants);
        assert_eq!(url, "https://www.example.com/p");
    }

    #[test]
    fn unconfigured_server_uses_request_host() {
        // Local development behind a tunnel: nothing configured, so the
        // tunnel host is the only information there is.
        let h = hdrs(&[("host", "abc123.ngrok.app"), ("x-forwarded-proto", "https")]);
        let url = request_public_url(&h, "/p?q=1", &cfg("", &[]), no_tenants);
        assert_eq!(url, "https://abc123.ngrok.app/p?q=1");
    }

    #[test]
    fn loopback_defaults_to_http_and_honors_forwarded_https() {
        let h = hdrs(&[("host", "localhost:4321")]);
        assert_eq!(
            request_public_url(&h, "/p", &cfg("https://app.example.com", &[]), no_tenants),
            "http://localhost:4321/p"
        );
        let h = hdrs(&[("host", "127.0.0.1:4321"), ("x-forwarded-proto", "https")]);
        assert_eq!(
            request_public_url(&h, "/p", &cfg("", &[]), no_tenants),
            "https://127.0.0.1:4321/p"
        );
    }

    #[test]
    fn non_loopback_trusted_host_defaults_to_https() {
        let h = hdrs(&[("host", "app.example.com")]);
        assert_eq!(
            request_public_url(&h, "/p", &cfg("", &["app.example.com"]), no_tenants),
            "https://app.example.com/p"
        );
    }

    #[test]
    fn forwarded_proto_other_than_http_or_https_is_ignored() {
        let h = hdrs(&[
            ("host", "app.example.com"),
            ("x-forwarded-proto", "javascript"),
        ]);
        assert_eq!(
            request_public_url(&h, "/p", &cfg("", &["app.example.com"]), no_tenants),
            "https://app.example.com/p"
        );
    }

    #[test]
    fn tenant_domain_is_trusted() {
        let h = hdrs(&[
            ("host", "shop.customer.com"),
            ("x-forwarded-proto", "https"),
        ]);
        let url = request_public_url(&h, "/p", &cfg("https://app.example.com", &[]), |h| {
            h == "shop.customer.com"
        });
        assert_eq!(url, "https://shop.customer.com/p");
    }

    #[test]
    fn malformed_host_is_never_used() {
        // A Host carrying URL syntax must not rewrite the URL's shape.
        let h = hdrs(&[("host", "evil.com/@app.example.com")]);
        assert_eq!(
            request_public_url(&h, "/p", &cfg("https://app.example.com", &[]), no_tenants),
            "https://app.example.com/p"
        );
        assert_eq!(
            request_public_url(&h, "/p", &cfg("", &[]), no_tenants),
            "/p"
        );
    }

    #[test]
    fn allowlist_matches_authority_with_port() {
        let c = cfg(
            "https://app.example.com:8443/base",
            &[" other.example.com "],
        );
        assert!(c.allows_host("app.example.com:8443"));
        assert!(!c.allows_host("app.example.com"));
        assert!(c.allows_host("other.example.com"));
        assert!(!c.allows_host(""));
    }
}
