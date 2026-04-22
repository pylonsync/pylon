use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// TLS configuration types
// ---------------------------------------------------------------------------

/// TLS configuration for production deployments.
///
/// statecraft itself runs plain HTTP (via tiny_http), so TLS termination is
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
        // Production config:
        //   - listen :80 only to redirect to HTTPS (no plain-HTTP routes).
        //   - TLS 1.2+ only; weak ciphers (RC4, 3DES, CBC) are gone from the
        //     TLSv1.2 default list on modern OpenSSL.
        //   - HSTS (1y + includeSubDomains + preload) on the TLS vhost.
        //   - Long proxy_read_timeout for SSE / WebSocket streams.
        //   - X-Forwarded-* headers so the app sees the real client IP.
        format!(
            r#"# Redirect plain HTTP to HTTPS.
server {{
    listen 80;
    listen [::]:80;
    return 301 https://$host$request_uri;
}}

server {{
    listen {ssl_port} ssl http2;
    listen [::]:{ssl_port} ssl http2;

    ssl_certificate {cert};
    ssl_certificate_key {key};
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_prefer_server_ciphers off;
    ssl_session_cache shared:SSL:10m;
    ssl_session_timeout 1d;
    ssl_session_tickets off;

    add_header Strict-Transport-Security "max-age=31536000; includeSubDomains; preload" always;
    add_header X-Content-Type-Options "nosniff" always;
    add_header X-Frame-Options "SAMEORIGIN" always;
    add_header Referrer-Policy "strict-origin-when-cross-origin" always;

    # SSE / fn streaming / AI streaming need long read windows — default 60s
    # chops live responses.
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;

    # Cap request bodies matching the server's 10 MB limit; nginx's default
    # is 1 MB and will 413 longer uploads before they reach the app.
    client_max_body_size 10M;

    location / {{
        proxy_pass http://127.0.0.1:{port};
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_buffering off; # required for SSE chunked responses
    }}

    location /ws {{
        proxy_pass http://127.0.0.1:{ws_port};
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 3600s;
        proxy_send_timeout 3600s;
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

    # Dev-only plain-HTTP snippet. For production, pass a TlsConfig so the
    # generator adds HSTS, TLS version pinning, and the HTTP -> HTTPS
    # redirect.

    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
    client_max_body_size 10M;

    location / {{
        proxy_pass http://127.0.0.1:{port};
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_buffering off;
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
    // Caddy auto-redirects HTTP to HTTPS and provisions certificates via
    // ACME when a domain is specified, so this config is short on purpose.
    // The extras here: HSTS, long flush interval for SSE, bumped read
    // timeout so streaming responses don't get cut at the default 30s.
    format!(
        r#"{domain} {{
    header {{
        Strict-Transport-Security "max-age=31536000; includeSubDomains; preload"
        X-Content-Type-Options "nosniff"
        X-Frame-Options "SAMEORIGIN"
        Referrer-Policy "strict-origin-when-cross-origin"
    }}

    # Match app body-size cap (10 MB).
    request_body {{
        max_size 10MB
    }}

    # Long-lived SSE / function streaming responses. Caddy's default
    # write_timeout would terminate live streams after 30s.
    servers {{
        timeouts {{
            read_body   30s
            read_header 10s
            write       1h
            idle        2m
        }}
    }}

    @websocket {{
        header Connection *Upgrade*
        header Upgrade websocket
    }}
    reverse_proxy @websocket localhost:{ws_port}

    reverse_proxy localhost:{port} {{
        flush_interval -1
        transport http {{
            read_timeout 1h
        }}
    }}
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

        assert!(config.contains("listen 443 ssl http2;"));
        assert!(config.contains("Strict-Transport-Security"));
        assert!(config.contains("TLSv1.2"));
        assert!(config.contains("return 301 https://"));
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

        assert!(config.contains("listen 8443 ssl http2;"));
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
