use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// TLS configuration types
// ---------------------------------------------------------------------------

/// TLS configuration for production deployments.
///
/// agentdb itself runs plain HTTP (via tiny_http), so TLS termination is
/// handled by a reverse proxy such as nginx or Caddy.  This struct captures
/// the certificate paths and listen port so we can generate working proxy
/// configs automatically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Path to the PEM-encoded certificate file.
    pub cert_path: String,
    /// Path to the PEM-encoded private key file.
    pub key_path: String,
    /// Port the reverse proxy should listen on for HTTPS traffic.
    pub port: u16,
}

// ---------------------------------------------------------------------------
// Reverse proxy config generators
// ---------------------------------------------------------------------------

/// Generate an nginx reverse proxy configuration.
///
/// When `tls` is `Some`, the config listens on the TLS port with
/// `ssl_certificate` / `ssl_certificate_key` directives.  When `None`, it
/// listens on port 80 in plain HTTP mode.
///
/// WebSocket traffic is routed to `app_port + 1`.
pub fn generate_nginx_config(app_port: u16, tls: Option<&TlsConfig>) -> String {
    if let Some(tls) = tls {
        format!(
            r#"server {{
    listen {ssl_port} ssl;
    ssl_certificate {cert};
    ssl_certificate_key {key};

    location / {{
        proxy_pass http://127.0.0.1:{port};
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }}

    location /ws {{
        proxy_pass http://127.0.0.1:{ws_port};
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }}
}}"#,
            ssl_port = tls.port,
            cert = tls.cert_path,
            key = tls.key_path,
            port = app_port,
            ws_port = app_port + 1,
        )
    } else {
        format!(
            r#"server {{
    listen 80;

    location / {{
        proxy_pass http://127.0.0.1:{port};
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }}
}}"#,
            port = app_port,
        )
    }
}

/// Generate a Caddy reverse proxy configuration with automatic TLS.
///
/// Caddy handles certificate provisioning via ACME, so only the domain
/// name is required.
pub fn generate_caddy_config(domain: &str, app_port: u16) -> String {
    format!(
        r#"{domain} {{
    reverse_proxy localhost:{port}

    @websocket {{
        header Connection *Upgrade*
        header Upgrade websocket
    }}
    reverse_proxy @websocket localhost:{ws_port}
}}"#,
        domain = domain,
        port = app_port,
        ws_port = app_port + 1,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nginx_config_with_tls() {
        let tls = TlsConfig {
            cert_path: "/etc/ssl/certs/app.pem".into(),
            key_path: "/etc/ssl/private/app.key".into(),
            port: 443,
        };
        let config = generate_nginx_config(4321, Some(&tls));

        assert!(config.contains("listen 443 ssl;"));
        assert!(config.contains("ssl_certificate /etc/ssl/certs/app.pem;"));
        assert!(config.contains("ssl_certificate_key /etc/ssl/private/app.key;"));
        assert!(config.contains("proxy_pass http://127.0.0.1:4321;"));
        assert!(config.contains("proxy_pass http://127.0.0.1:4322;"));
        assert!(config.contains("X-Forwarded-Proto"));
    }

    #[test]
    fn nginx_config_without_tls() {
        let config = generate_nginx_config(4321, None);

        assert!(config.contains("listen 80;"));
        assert!(config.contains("proxy_pass http://127.0.0.1:4321;"));
        assert!(!config.contains("ssl_certificate"));
        assert!(!config.contains("443"));
    }

    #[test]
    fn caddy_config_contains_domain_and_ports() {
        let config = generate_caddy_config("example.com", 4321);

        assert!(config.contains("example.com {"));
        assert!(config.contains("reverse_proxy localhost:4321"));
        assert!(config.contains("reverse_proxy @websocket localhost:4322"));
        assert!(config.contains("header Upgrade websocket"));
    }

    #[test]
    fn nginx_config_correct_ws_port() {
        let tls = TlsConfig {
            cert_path: "/cert.pem".into(),
            key_path: "/key.pem".into(),
            port: 8443,
        };
        let config = generate_nginx_config(9000, Some(&tls));

        assert!(config.contains("listen 8443 ssl;"));
        assert!(config.contains("proxy_pass http://127.0.0.1:9000;"));
        assert!(config.contains("proxy_pass http://127.0.0.1:9001;"));
    }

    #[test]
    fn tls_config_serialization_roundtrip() {
        let tls = TlsConfig {
            cert_path: "/cert.pem".into(),
            key_path: "/key.pem".into(),
            port: 443,
        };
        let json = serde_json::to_string(&tls).unwrap();
        let parsed: TlsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.cert_path, "/cert.pem");
        assert_eq!(parsed.key_path, "/key.pem");
        assert_eq!(parsed.port, 443);
    }
}
