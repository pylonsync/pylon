//! Native sign-in to a federated app, without a browser.
//!
//! A native app signs in to the IdP (here a dev session stands in for
//! Apple, Google or an email code), trades that session at the IdP's
//! `/oidc/token` with an RFC 8693 token exchange for an id_token addressed
//! to the client app, and posts it to the client's
//! `POST /api/auth/federated/<provider>`. The client verifies the token
//! against the IdP's JWKS and signs the user in through the same path as
//! the browser callback, org federation mirror included.
//!
//! Like oidc_self_federation.rs, one server plays both halves.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;

fn string_field(name: &str, optional: bool, unique: bool) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional,
        unique,
        crdt: None,
        server_only: false,
        readonly: false,
        default: None,
        enum_values: None,
        encrypted: false,
        sync_omit: false,
        max_length: None,
    }
}

fn entity(name: &str, fields: Vec<ManifestField>) -> ManifestEntity {
    ManifestEntity {
        name: name.into(),
        fields,
        indexes: vec![],
        relations: vec![],
        search: None,
        crdt: true,
        sync: true,
        ..Default::default()
    }
}

fn test_manifest() -> AppManifest {
    AppManifest {
        required_env: Vec::new(),
        build: Default::default(),
        shards: Vec::new(),
        manifest_version: 1,
        name: "oidc-native-exchange".into(),
        version: "0.1.0".into(),
        entities: vec![
            entity(
                "User",
                vec![
                    string_field("email", false, true),
                    string_field("displayName", true, false),
                    // A timestamp, as auth writes it; the IdP reads it for
                    // the email_verified claim.
                    string_field("emailVerified", true, false),
                ],
            ),
            // Org/OrgMember/OrgInvite in the framework-required shapes
            // (org.rs "Required entity shapes") — the `orgs` claim reads
            // through the same OrgStore as /api/auth/orgs.
            entity(
                "Org",
                vec![
                    string_field("name", false, false),
                    string_field("createdBy", false, false),
                    string_field("createdAt", false, false),
                    // Federation: the mirror of an upstream org is keyed here.
                    string_field("externalId", true, true),
                ],
            ),
            entity(
                "OrgMember",
                vec![
                    string_field("orgId", false, false),
                    string_field("userId", false, false),
                    string_field("role", false, false),
                    string_field("joinedAt", false, false),
                ],
            ),
            entity(
                "OrgInvite",
                vec![
                    string_field("orgId", false, false),
                    string_field("email", false, false),
                    string_field("role", false, false),
                    string_field("invitedBy", false, false),
                    string_field("tokenHash", false, false),
                    string_field("tokenPrefix", false, false),
                    string_field("createdAt", false, false),
                    string_field("expiresAt", false, false),
                    string_field("acceptedAt", true, false),
                ],
            ),
        ],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: {
            let mut auth = pylon_kernel::ManifestAuthConfig::default();
            // The client half mirrors orgs from the `selfidp` provider — in
            // this self-federated loop, from its own IdP. The IdP's org and
            // its local mirror are DIFFERENT rows in the same table; the
            // mirror is the one with `externalId` set.
            auth.org.federation = Some(pylon_kernel::ManifestAuthOrgFederation {
                provider: "selfidp".into(),
                external_id_field: "externalId".into(),
                remove_missing: true,
                role_map: Default::default(),
                // The test creates the IdP-side org through POST /api/auth/orgs,
                // which the guard would refuse; leave local create enabled.
                disable_local_create: false,
                slug_field: None,
            });
            auth
        },
        llm: Default::default(),
        connections: vec![],
        crons: vec![],
        fonts: vec![],
    }
}

fn available_port() -> u16 {
    // Own lane so parallel test binaries can't collide (oidc_self_federation
    // takes 25_000 to 26_000 in steps of 4; this takes the odd slots).
    for base in (25_002..26_000).step_by(4) {
        if pylon_runtime::listen::port_is_free(base) {
            return base;
        }
    }
    panic!("no free port in the 25000-26000 lane");
}

/// Minimal raw-TCP request helper. Returns (status, set_cookies, headers-as-
/// lowercased-pairs, body). Raw TCP rather than a client lib so redirects are
/// NEVER followed — every 302 in this flow is an assertion target.
fn http_request(
    method: &str,
    url: &str,
    body: Option<&str>,
    extra_headers: &[(&str, &str)],
) -> (u16, Vec<String>, Vec<(String, String)>, String) {
    let host = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match host.find('/') {
        Some(i) => (&host[..i], &host[i..]),
        None => (host, "/"),
    };
    let body_str = body.unwrap_or("");
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nOrigin: http://{host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body_str.len()
    );
    for (k, v) in extra_headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body_str);

    let mut stream = TcpStream::connect(host_port).expect("connect");
    // Generous: the callback request performs a discovery fetch + token
    // exchange + userinfo against this same server before answering.
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.write_all(req.as_bytes()).expect("write");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let text = String::from_utf8_lossy(&response).to_string();

    let status: u16 = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut set_cookies = Vec::new();
    let mut headers = Vec::new();
    for line in text.lines().skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim().to_string();
            if key == "set-cookie" {
                set_cookies.push(val.clone());
            }
            headers.push((key, val));
        }
    }
    let body_out = match text.find("\r\n\r\n") {
        Some(i) => text[i + 4..].to_string(),
        None => String::new(),
    };
    (status, set_cookies, headers, body_out)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// `name=value` pairs from Set-Cookie lines, joined for a Cookie header.
fn cookie_jar(set_cookies: &[String]) -> String {
    set_cookies
        .iter()
        .filter_map(|c| c.split(';').next())
        .collect::<Vec<_>>()
        .join("; ")
}

fn query_param(url: &str, name: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

#[test]
fn native_app_signs_in_to_a_federated_app_through_token_exchange() {
    let port = available_port();
    let origin = format!("http://127.0.0.1:{port}");
    let redirect_uri = format!("{origin}/api/auth/callback/selfidp");
    let key_path = std::env::temp_dir().join(format!("pylon-oidc-exchange-{port}.pem"));

    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_ADMIN_TOKEN", "exchange-admin-token");
        std::env::set_var("PYLON_OIDC_ISSUER", &origin);
        std::env::set_var(
            "PYLON_OIDC_CLIENTS",
            format!(
                r#"[{{"client_id":"self-app","client_secret":"s3cr3t-exchange","redirect_uris":["{redirect_uri}"],"token_exchange":true}},{{"client_id":"web-only","client_secret":"other","redirect_uris":[]}}]"#
            ),
        );
        std::env::set_var("PYLON_OIDC_KEY_PATH", &key_path);
        std::env::set_var("PYLON_LOGIN_URL", "/login");
        std::env::set_var("PYLON_OAUTH_SELFIDP_OIDC_ISSUER", &origin);
        std::env::set_var("PYLON_OAUTH_SELFIDP_CLIENT_ID", "self-app");
        std::env::set_var("PYLON_OAUTH_SELFIDP_CLIENT_SECRET", "s3cr3t-exchange");
        std::env::set_var("PYLON_OAUTH_SELFIDP_REDIRECT", &redirect_uri);
    }
    pylon_auth::oidc_provider::OidcKeyStore::load_or_generate(&key_path)
        .expect("pre-generate OIDC signing key");

    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let rt2 = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt2, port);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while TcpStream::connect(format!("127.0.0.1:{port}")).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "server never bound {port}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // ── The user, a session on the IdP, and an IdP org. ─────────────────
    let (status, _, _, body) = http_request(
        "POST",
        &format!("{origin}/api/entities/User"),
        Some(
            r#"{"email":"native@example.com","displayName":"Native User","emailVerified":"2026-10-15T00:00:00Z"}"#,
        ),
        &[("Authorization", "Bearer exchange-admin-token")],
    );
    assert_eq!(status, 201, "create User failed: {body}");
    let user: serde_json::Value = serde_json::from_str(&body).unwrap();
    let user_id = user
        .get("id")
        .or_else(|| user.get("data").and_then(|d| d.get("id")))
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    let (status, _, _, body) = http_request(
        "POST",
        &format!("{origin}/api/auth/session"),
        Some(&format!(r#"{{"user_id":"{user_id}"}}"#)),
        &[],
    );
    assert_eq!(status, 201, "dev session mint failed: {body}");
    let idp_token = serde_json::from_str::<serde_json::Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let idp_bearer = format!("Bearer {idp_token}");
    let (status, _, _, body) = http_request(
        "POST",
        &format!("{origin}/api/auth/orgs"),
        Some(r#"{"name":"Native Org"}"#),
        &[("Authorization", idp_bearer.as_str())],
    );
    assert_eq!(status, 200, "org create failed: {body}");

    let exchange = |client_id: &str, subject: &str, extra: &str| {
        http_request(
            "POST",
            &format!("{origin}/oidc/token"),
            Some(&format!(
                "grant_type={}&client_id={client_id}&subject_token={subject}&subject_token_type={}{extra}",
                url_encode("urn:ietf:params:oauth:grant-type:token-exchange"),
                url_encode("urn:ietf:params:oauth:token-type:access_token"),
            )),
            &[("Content-Type", "application/x-www-form-urlencoded")],
        )
    };

    // ── Refusals. ───────────────────────────────────────────────────────
    let (status, _, _, body) = exchange("web-only", &idp_token, "");
    assert_eq!(status, 400, "a client without token_exchange: {body}");
    assert!(body.contains("unauthorized_client"), "{body}");
    let (status, _, _, body) = exchange("self-app", "not-a-session", "");
    assert_eq!(status, 400, "an unknown subject token: {body}");
    assert!(body.contains("invalid_grant"), "{body}");
    let (status, _, _, body) = exchange("self-app", &idp_token, "&scope=email%20orgs");
    assert_eq!(status, 400, "scope without openid: {body}");
    assert!(body.contains("invalid_scope"), "{body}");
    let (status, _, _, body) = exchange("nobody", &idp_token, "");
    assert_eq!(status, 401, "an unknown client: {body}");

    // ── The exchange: an id_token for the client, with the orgs claim. ──
    let (status, _, _, body) = exchange("self-app", &idp_token, "");
    assert_eq!(status, 200, "token exchange: {body}");
    let tokens: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        tokens["issued_token_type"].as_str(),
        Some("urn:ietf:params:oauth:token-type:access_token")
    );
    let id_token = tokens["id_token"].as_str().expect("id_token").to_string();
    let access_token = tokens["access_token"]
        .as_str()
        .expect("access_token")
        .to_string();
    let claims: serde_json::Value =
        serde_json::from_slice(&b64url_decode(id_token.split('.').nth(1).unwrap())).unwrap();
    assert_eq!(claims["aud"].as_str(), Some("self-app"));
    assert_eq!(claims["sub"].as_str(), Some(user_id.as_str()));
    assert_eq!(claims["email"].as_str(), Some("native@example.com"));
    assert!(
        claims["orgs"]
            .as_array()
            .is_some_and(|o| o.iter().any(|x| x["name"] == "Native Org")),
        "orgs claim: {claims}"
    );

    // ── Federated sign-in on the client. ────────────────────────────────
    let (status, _, _, body) = http_request(
        "POST",
        &format!("{origin}/api/auth/federated/nosuch"),
        Some(&format!(r#"{{"id_token":"{id_token}"}}"#)),
        &[],
    );
    assert_eq!(status, 404, "an unconfigured provider: {body}");

    let mut parts: Vec<String> = id_token.split('.').map(String::from).collect();
    let mut forged = claims.clone();
    forged["email"] = serde_json::Value::String("someone-else@example.com".into());
    parts[1] = base64_url(forged.to_string().as_bytes());
    let (status, _, _, body) = http_request(
        "POST",
        &format!("{origin}/api/auth/federated/selfidp"),
        Some(&format!(r#"{{"id_token":"{}"}}"#, parts.join("."))),
        &[],
    );
    assert_eq!(status, 401, "a tampered token: {body}");

    let (status, _, _, body) = http_request(
        "POST",
        &format!("{origin}/api/auth/federated/selfidp"),
        Some(&format!(
            r#"{{"id_token":"{id_token}","access_token":"{access_token}"}}"#
        )),
        &[],
    );
    assert_eq!(status, 200, "federated sign-in: {body}");
    let signed_in: serde_json::Value = serde_json::from_str(&body).unwrap();
    let app_token = signed_in["token"]
        .as_str()
        .expect("session token")
        .to_string();
    assert_eq!(signed_in["provider"].as_str(), Some("selfidp"));
    let app_bearer = format!("Bearer {app_token}");

    let (status, _, _, body) = http_request(
        "GET",
        &format!("{origin}/api/auth/me"),
        None,
        &[("Authorization", app_bearer.as_str())],
    );
    assert_eq!(status, 200, "/api/auth/me with the app session: {body}");
    let me: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        me.get("user_id")
            .or_else(|| me.get("userId"))
            .and_then(|v| v.as_str()),
        Some(user_id.as_str()),
        "the same user as on the IdP: {me}"
    );

    // The org mirror ran: the IdP org and its local mirror.
    let (status, _, _, body) = http_request(
        "GET",
        &format!("{origin}/api/auth/orgs"),
        None,
        &[("Authorization", app_bearer.as_str())],
    );
    assert_eq!(status, 200, "list orgs: {body}");
    let listed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let list = listed
        .as_array()
        .or_else(|| listed.get("orgs").and_then(|v| v.as_array()))
        .expect("orgs array");
    assert_eq!(list.len(), 2, "IdP org + its local mirror: {listed}");
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn base64_url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn b64url_decode(s: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .expect("b64url")
}
