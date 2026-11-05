//! Refresh tokens, revocation, and the function auth ops of Pylon's OIDC
//! provider, against a real server.
//!
//! One server is both halves, as in `oidc_self_federation.rs`: the IdP
//! (`PYLON_OIDC_ISSUER`) and a Pylon app signing in against it
//! (`PYLON_OAUTH_SELFIDP_*`, which asks for `offline_access` because the
//! IdP advertises it). The tests drive
//! `/oidc/token`, `/oidc/revoke`, and `/oidc/userinfo` as a raw client,
//! the app's own sign-in and refresh path, and the host side of
//! `ctx.auth.providerToken` / `ctx.auth.verifyOidcToken`
//! (`pylon_runtime::app_auth_ops`) in this process, which shares the
//! provider singleton and the OAuth registry with the server.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;

const ADMIN: &str = "Bearer refresh-admin-token";
const CLIENT: &str = "self-app";
const SECRET: &str = "s3cr3t-refresh";
const OTHER_CLIENT: &str = "other-app";
const OTHER_SECRET: &str = "other-s3cr3t";

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

fn test_manifest() -> AppManifest {
    AppManifest {
        required_env: Vec::new(),
        build: Default::default(),
        shards: Vec::new(),
        offline: None,
        manifest_version: 1,
        name: "oidc-refresh".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "User".into(),
            fields: vec![
                string_field("email", false, true),
                string_field("displayName", true, false),
                string_field("emailVerified", true, false),
            ],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
            sync: true,
            ..Default::default()
        }],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
        llm: Default::default(),
        connections: vec![],
        crons: vec![],
        fonts: vec![],
    }
}

/// The server's origin. Booted once per test binary: the OIDC provider,
/// its token store, and the OAuth registry are process singletons.
fn origin() -> &'static str {
    static ORIGIN: OnceLock<String> = OnceLock::new();
    ORIGIN.get_or_init(|| {
        let port = (14_000..14_400)
            .step_by(4)
            .find(|p| pylon_runtime::listen::port_is_free(*p))
            .expect("no free port in the 14000-14400 lane");
        let origin = format!("http://127.0.0.1:{port}");
        let redirect_uri = format!("{origin}/api/auth/callback/selfidp");
        let key_path = std::env::temp_dir().join(format!("pylon-oidc-refresh-{port}.pem"));
        // SAFETY: inside the OnceLock initializer, before any server thread
        // or other test reads the environment.
        unsafe {
            std::env::set_var("PYLON_DEV_MODE", "1");
            std::env::set_var("PYLON_ADMIN_TOKEN", ADMIN.trim_start_matches("Bearer "));
            std::env::set_var("PYLON_OIDC_ISSUER", &origin);
            std::env::set_var(
                "PYLON_OIDC_CLIENTS",
                format!(
                    r#"[{{"client_id":"{CLIENT}","client_secret":"{SECRET}","redirect_uris":["{redirect_uri}"],"token_exchange":true}},
                        {{"client_id":"{OTHER_CLIENT}","client_secret":"{OTHER_SECRET}","redirect_uris":["{redirect_uri}"]}}]"#
                ),
            );
            std::env::set_var("PYLON_OIDC_KEY_PATH", &key_path);
            std::env::set_var("PYLON_LOGIN_URL", "/login");
            std::env::set_var("PYLON_OAUTH_SELFIDP_OIDC_ISSUER", &origin);
            std::env::set_var("PYLON_OAUTH_SELFIDP_CLIENT_ID", CLIENT);
            std::env::set_var("PYLON_OAUTH_SELFIDP_CLIENT_SECRET", SECRET);
            std::env::set_var("PYLON_OAUTH_SELFIDP_REDIRECT", &redirect_uri);
            // No PYLON_OAUTH_SELFIDP_SCOPES: the client asks for
            // offline_access because this Pylon IdP advertises it.
            // Every /api/auth/oauth/refresh call refreshes (tokens live 1h).
            std::env::set_var("PYLON_OAUTH_REFRESH_BUFFER_SECS", "7200");
        }
        pylon_auth::oidc_provider::OidcKeyStore::load_or_generate(&key_path)
            .expect("pre-generate OIDC signing key");

        let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
        std::thread::spawn(move || {
            let _ = pylon_runtime::server::start(rt, port);
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while TcpStream::connect(format!("127.0.0.1:{port}")).is_err() {
            assert!(
                std::time::Instant::now() < deadline,
                "server never bound 127.0.0.1:{port}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        origin
    })
}

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    set_cookies: Vec<String>,
    body: String,
}

impl Response {
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Raw-TCP request so no redirect is ever followed.
fn request(method: &str, path: &str, body: Option<&str>, headers: &[(&str, &str)]) -> Response {
    let host = origin().trim_start_matches("http://");
    let body = body.unwrap_or("");
    let has_type = headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if !has_type {
        req.push_str("Content-Type: application/json\r\n");
    }
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    let mut stream = TcpStream::connect(host).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.write_all(req.as_bytes()).expect("write");
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).to_string();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut out_headers = Vec::new();
    let mut set_cookies = Vec::new();
    for line in text.lines().skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            if key == "set-cookie" {
                set_cookies.push(v.trim().to_string());
            }
            out_headers.push((key, v.trim().to_string()));
        }
    }
    let body = text
        .find("\r\n\r\n")
        .map(|i| text[i + 4..].to_string())
        .unwrap_or_default();
    Response {
        status,
        headers: out_headers,
        set_cookies,
        body,
    }
}

fn form_post(path: &str, body: &str, extra: &[(&str, &str)]) -> Response {
    let mut headers = vec![("Content-Type", "application/x-www-form-urlencoded")];
    headers.extend_from_slice(extra);
    request("POST", path, Some(body), &headers)
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn query_param(url: &str, name: &str) -> Option<String> {
    url.split_once('?')?.1.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

/// A user on the IdP and a session for them there.
struct Person {
    user_id: String,
    session: String,
}

fn person(email: &str) -> Person {
    origin();
    let r = request(
        "POST",
        "/api/entities/User",
        Some(&format!(
            r#"{{"email":"{email}","displayName":"Test Person","emailVerified":"2026-01-01T00:00:00Z"}}"#
        )),
        &[("Authorization", ADMIN)],
    );
    assert_eq!(r.status, 201, "create user: {}", r.body);
    let v = r.json();
    let user_id = v
        .get("id")
        .or_else(|| v.get("data").and_then(|d| d.get("id")))
        .and_then(|x| x.as_str())
        .expect("user id")
        .to_string();
    let r = request(
        "POST",
        "/api/auth/session",
        Some(&format!(r#"{{"user_id":"{user_id}"}}"#)),
        &[],
    );
    assert_eq!(r.status, 201, "dev session: {}", r.body);
    let session = r.json()["token"].as_str().expect("token").to_string();
    Person { user_id, session }
}

fn redirect_uri() -> String {
    format!("{}/api/auth/callback/selfidp", origin())
}

/// Authorize as `p` and exchange the code at /oidc/token as a raw client.
fn code_flow(p: &Person, scope: &str) -> serde_json::Value {
    let verifier = "refresh-test-verifier-0123456789abcdefghijklmnopq";
    let challenge = {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier))
    };
    let path = format!(
        "/oidc/authorize?client_id={CLIENT}&redirect_uri={}&response_type=code&scope={}&code_challenge={challenge}&code_challenge_method=S256&state=s",
        url_encode(&redirect_uri()),
        url_encode(scope),
    );
    let bearer = format!("Bearer {}", p.session);
    let r = request("GET", &path, None, &[("Authorization", bearer.as_str())]);
    assert_eq!(r.status, 302, "authorize: {}", r.body);
    let code = query_param(r.header("location").expect("Location"), "code").expect("code");
    let r = form_post(
        "/oidc/token",
        &format!(
            "grant_type=authorization_code&code={code}&client_id={CLIENT}&client_secret={SECRET}&redirect_uri={}&code_verifier={verifier}",
            url_encode(&redirect_uri())
        ),
        &[],
    );
    assert_eq!(r.status, 200, "code exchange: {}", r.body);
    r.json()
}

fn refresh(refresh_token: &str, client: &str, secret: &str, scope: Option<&str>) -> Response {
    let mut body = format!(
        "grant_type=refresh_token&refresh_token={}&client_id={client}&client_secret={secret}",
        url_encode(refresh_token)
    );
    if let Some(s) = scope {
        body.push_str(&format!("&scope={}", url_encode(s)));
    }
    form_post("/oidc/token", &body, &[])
}

fn userinfo_status(access_token: &str) -> u16 {
    let bearer = format!("Bearer {access_token}");
    request(
        "GET",
        "/oidc/userinfo",
        None,
        &[("Authorization", bearer.as_str())],
    )
    .status
}

fn str_of(v: &serde_json::Value, key: &str) -> String {
    v[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} missing: {v}"))
        .to_string()
}

fn provider() -> Arc<pylon_auth::oidc_provider::OidcProvider> {
    pylon_auth::oidc_provider::provider_or_init().expect("this process is an IdP")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn discovery_advertises_offline_access_refresh_and_revocation() {
    origin();
    let doc = request("GET", "/.well-known/openid-configuration", None, &[]).json();
    let has = |key: &str, val: &str| {
        doc[key]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| x == val))
    };
    assert!(has("scopes_supported", "offline_access"), "{doc}");
    assert!(has("grant_types_supported", "refresh_token"), "{doc}");
    assert_eq!(
        doc["revocation_endpoint"],
        format!("{}/oidc/revoke", origin())
    );
}

#[test]
fn refresh_token_only_with_offline_access() {
    let p = person("no-offline@example.com");
    let tokens = code_flow(&p, "openid email");
    assert!(tokens.get("refresh_token").is_none(), "{tokens}");
    let tokens = code_flow(&p, "openid email offline_access");
    assert!(tokens["refresh_token"].is_string(), "{tokens}");
    assert_eq!(tokens["refresh_token_expires_in"], 30 * 24 * 3600);
    assert_eq!(tokens["expires_in"], 3600);
}

#[test]
fn refresh_rotates_and_keeps_the_grant() {
    let p = person("rotate@example.com");
    let first = code_flow(&p, "openid email offline_access");
    let rt1 = str_of(&first, "refresh_token");

    let r = refresh(&rt1, CLIENT, SECRET, None);
    assert_eq!(r.status, 200, "refresh: {}", r.body);
    let second = r.json();
    let rt2 = str_of(&second, "refresh_token");
    assert_ne!(rt1, rt2, "every refresh issues a new refresh token");
    assert_ne!(
        str_of(&first, "access_token"),
        str_of(&second, "access_token")
    );
    assert_eq!(second["scope"], "openid email offline_access");
    assert!(second["id_token"].is_string());
    assert_eq!(userinfo_status(&str_of(&second, "access_token")), 200);

    // Same grant: both access tokens resolve to one grant id.
    let p1 = provider()
        .lookup_access_token(&str_of(&first, "access_token"))
        .unwrap()
        .unwrap();
    let p2 = provider()
        .lookup_access_token(&str_of(&second, "access_token"))
        .unwrap()
        .unwrap();
    assert_eq!(p1.grant_id, p2.grant_id);
    assert_eq!(p2.user_id, p.user_id);

    // The rotated token, again within the grace window: still accepted
    // (two requests of one app refreshing at once).
    let r = refresh(&rt1, CLIENT, SECRET, None);
    assert_eq!(r.status, 200, "grace reuse: {}", r.body);

    // Basic client authentication works the same way.
    use base64::Engine;
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT}:{SECRET}"))
    );
    let r = form_post(
        "/oidc/token",
        &format!(
            "grant_type=refresh_token&refresh_token={}",
            url_encode(&rt2)
        ),
        &[("Authorization", basic.as_str())],
    );
    assert_eq!(r.status, 200, "basic auth refresh: {}", r.body);
}

#[test]
fn late_reuse_of_a_rotated_refresh_token_revokes_the_grant() {
    let p = person("theft@example.com");
    let first = code_flow(&p, "openid offline_access");
    let rt1 = str_of(&first, "refresh_token");
    let second = refresh(&rt1, CLIENT, SECRET, None).json();
    // Move the rotation two minutes into the past: the store keeps the
    // first rotation time, so rewrite the row.
    let store = provider().tokens.clone();
    let mut row = store
        .find(&pylon_auth::oidc_tokens::hash_token(&rt1), now())
        .unwrap()
        .expect("a rotated token stays in the store");
    assert!(row.rotated_at.is_some());
    row.rotated_at = Some(now() - 120);
    store.revoke_grant(&row.grant_id).unwrap();
    store.insert(&row).unwrap();
    // A live token of the same grant, for the theft response to remove.
    let live = provider()
        .issue_tokens(
            Some(&row.grant_id),
            &p.user_id,
            CLIENT,
            &row.scopes,
            Some(&row.scopes),
        )
        .unwrap();
    assert_eq!(userinfo_status(&live.access_token), 200);

    let r = refresh(&rt1, CLIENT, SECRET, None);
    assert_eq!(r.status, 400, "{}", r.body);
    assert!(r.body.contains("invalid_grant"), "{}", r.body);
    assert_eq!(userinfo_status(&live.access_token), 401, "grant revoked");
    let r = refresh(live.refresh_token.as_deref().unwrap(), CLIENT, SECRET, None);
    assert_eq!(r.status, 400, "the whole grant is gone: {}", r.body);
    // The token the honest refresh got is part of the grant too.
    assert_eq!(userinfo_status(&str_of(&second, "access_token")), 401);
}

#[test]
fn refresh_is_bound_to_its_client_and_scopes() {
    let p = person("bound@example.com");
    let tokens = code_flow(&p, "openid email offline_access");
    let rt = str_of(&tokens, "refresh_token");

    // Another registered client cannot use it.
    let r = refresh(&rt, OTHER_CLIENT, OTHER_SECRET, None);
    assert_eq!(r.status, 400, "{}", r.body);
    assert!(r.body.contains("invalid_grant"));
    // A wrong secret is a client authentication failure.
    let r = refresh(&rt, CLIENT, "wrong", None);
    assert_eq!(r.status, 401, "{}", r.body);
    assert!(r.body.contains("invalid_client"));
    // Scopes may not widen.
    let r = refresh(
        &rt,
        CLIENT,
        SECRET,
        Some("openid email profile offline_access"),
    );
    assert_eq!(r.status, 400, "{}", r.body);
    assert!(r.body.contains("invalid_scope"));
    // They may narrow; the refresh token keeps the grant's scopes.
    let r = refresh(&rt, CLIENT, SECRET, Some("openid"));
    assert_eq!(r.status, 200, "{}", r.body);
    let narrowed = r.json();
    assert_eq!(narrowed["scope"], "openid");
    let r = refresh(&str_of(&narrowed, "refresh_token"), CLIENT, SECRET, None);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["scope"], "openid email offline_access");
    // The access token is not a refresh token.
    let r = refresh(&str_of(&tokens, "access_token"), CLIENT, SECRET, None);
    assert_eq!(r.status, 400, "{}", r.body);
}

#[test]
fn revoke_endpoint_revokes_the_grant() {
    let p = person("revoke@example.com");
    let tokens = code_flow(&p, "openid offline_access");
    let access = str_of(&tokens, "access_token");
    let rt = str_of(&tokens, "refresh_token");

    // Client authentication is required.
    let r = form_post("/oidc/revoke", &format!("token={access}"), &[]);
    assert_eq!(r.status, 401, "{}", r.body);
    // Another client's revoke is a no-op that still answers 200.
    let r = form_post(
        "/oidc/revoke",
        &format!("token={access}&client_id={OTHER_CLIENT}&client_secret={OTHER_SECRET}"),
        &[],
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(userinfo_status(&access), 200);
    // Unknown tokens: 200.
    let r = form_post(
        "/oidc/revoke",
        &format!("token=nope&client_id={CLIENT}&client_secret={SECRET}"),
        &[],
    );
    assert_eq!(r.status, 200, "{}", r.body);
    // Revoking the access token revokes the refresh token with it.
    let r = form_post(
        "/oidc/revoke",
        &format!(
            "token={access}&token_type_hint=access_token&client_id={CLIENT}&client_secret={SECRET}"
        ),
        &[],
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(userinfo_status(&access), 401);
    assert_eq!(refresh(&rt, CLIENT, SECRET, None).status, 400);
}

#[test]
fn deleting_the_user_revokes_their_grants() {
    let p = person("deleted@example.com");
    let tokens = code_flow(&p, "openid offline_access");
    let r = request(
        "DELETE",
        &format!("/api/entities/User/{}", p.user_id),
        None,
        &[("Authorization", ADMIN)],
    );
    assert_eq!(r.status, 200, "delete user: {}", r.body);
    assert_eq!(userinfo_status(&str_of(&tokens, "access_token")), 401);
    let r = refresh(&str_of(&tokens, "refresh_token"), CLIENT, SECRET, None);
    assert_eq!(r.status, 400, "{}", r.body);
}

#[test]
fn refresh_for_a_user_row_that_is_gone_is_refused() {
    // A grant whose user row disappears without the cascade (removed some
    // other way): the refresh checks the row itself.
    origin();
    let s = vec!["openid".to_string(), "offline_access".to_string()];
    let ghost = provider()
        .issue_tokens(None, "user-that-never-existed", CLIENT, &s, Some(&s))
        .unwrap();
    let r = refresh(
        ghost.refresh_token.as_deref().unwrap(),
        CLIENT,
        SECRET,
        None,
    );
    assert_eq!(r.status, 400, "{}", r.body);
    assert!(r.body.contains("invalid_grant"));
    assert!(provider()
        .lookup_access_token(&ghost.access_token)
        .unwrap()
        .is_none());
}

#[test]
fn signing_out_everywhere_revokes_grants() {
    let p = person("signout@example.com");
    let tokens = code_flow(&p, "openid offline_access");
    let bearer = format!("Bearer {}", p.session);
    let r = request(
        "DELETE",
        "/api/auth/sessions",
        None,
        &[("Authorization", bearer.as_str())],
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["revoked_oidc_grants"], 1, "{}", r.body);
    assert_eq!(userinfo_status(&str_of(&tokens, "access_token")), 401);
}

/// The app half: sign in with `offline_access`, then refresh through the
/// app's account store.
#[test]
fn app_sign_in_stores_and_rotates_the_refresh_token() {
    let p = person("app@example.com");
    let r = request(
        "GET",
        &format!(
            "/api/auth/login/selfidp?callback={0}/&error_callback={0}/login",
            origin()
        ),
        None,
        &[],
    );
    assert_eq!(r.status, 200, "login kickoff: {}", r.body);
    let authorize = r.json()["redirect"].as_str().unwrap().to_string();
    assert!(
        query_param(&authorize, "scope")
            .unwrap_or_default()
            .contains("offline_access"),
        "a Pylon IdP's offline_access is requested without app config: {authorize}"
    );
    let jar: String = r
        .set_cookies
        .iter()
        .filter_map(|c| c.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");
    let path = authorize.trim_start_matches(origin());
    let bearer = format!("Bearer {}", p.session);
    let r = request("GET", path, None, &[("Authorization", bearer.as_str())]);
    assert_eq!(r.status, 302, "authorize: {}", r.body);
    let callback = r
        .header("location")
        .unwrap()
        .trim_start_matches(origin())
        .to_string();
    let cb_headers: Vec<(&str, &str)> = if jar.is_empty() {
        vec![]
    } else {
        vec![("Cookie", jar.as_str())]
    };
    let r = request("GET", &callback, None, &cb_headers);
    assert!(
        r.status == 302 || r.status == 200,
        "callback: {} {}",
        r.status,
        r.body
    );
    let app_jar: String = r
        .set_cookies
        .iter()
        .filter_map(|c| c.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");
    assert!(!app_jar.is_empty(), "app session cookie");

    let account = || {
        let r = request(
            "GET",
            "/api/admin/auth/accounts",
            None,
            &[("Authorization", ADMIN)],
        );
        assert_eq!(r.status, 200, "{}", r.body);
        r.json()
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["provider_id"] == "selfidp" && a["user_id"] == p.user_id.as_str())
            .cloned()
            .unwrap_or_else(|| panic!("selfidp account of {}: {}", p.user_id, r.body))
    };
    let linked = account();
    assert_eq!(linked["has_refresh_token"], true, "{linked}");
    let rt_exp = linked["refresh_token_expires_at"]
        .as_u64()
        .expect("expiry stored");
    assert!(rt_exp > now() + 29 * 24 * 3600, "{linked}");

    // Refresh through the app (the buffer forces it): a new access token
    // each time, and each refresh stores the rotated refresh token.
    let cookie = [("Cookie", app_jar.as_str())];
    let a = request(
        "POST",
        "/api/auth/oauth/refresh/selfidp",
        Some("{}"),
        &cookie,
    );
    assert_eq!(a.status, 200, "{}", a.body);
    let b = request(
        "POST",
        "/api/auth/oauth/refresh/selfidp",
        Some("{}"),
        &cookie,
    );
    assert_eq!(b.status, 200, "{}", b.body);
    assert_ne!(a.json()["access_token"], b.json()["access_token"]);
    let after = account();
    assert!(after["updated_at"].as_u64() >= linked["updated_at"].as_u64());
    assert!(after["refresh_token_expires_at"].as_u64().unwrap() >= rt_exp);
    let access = b.json()["access_token"].as_str().unwrap().to_string();
    let record = provider().lookup_access_token(&access).unwrap().unwrap();
    assert_eq!(record.user_id, p.user_id);
    assert_eq!(record.client_id, CLIENT);

    // A later native sign-in brings no refresh token: the stored one stays,
    // and the access token it brings gets an expiry (here the id_token's,
    // since the body leaves out expires_in).
    let r = form_post(
        "/oidc/token",
        &format!(
            "grant_type={}&client_id={CLIENT}&subject_token={}&subject_token_type={}",
            url_encode("urn:ietf:params:oauth:grant-type:token-exchange"),
            p.session,
            url_encode("urn:ietf:params:oauth:token-type:access_token"),
        ),
        &[],
    );
    assert_eq!(r.status, 200, "token exchange: {}", r.body);
    let exchanged = r.json();
    assert!(exchanged.get("refresh_token").is_none(), "{exchanged}");
    let body = serde_json::json!({
        "id_token": exchanged["id_token"],
        "access_token": exchanged["access_token"],
    })
    .to_string();
    let r = request("POST", "/api/auth/federated/selfidp", Some(&body), &[]);
    assert_eq!(r.status, 200, "federated sign-in: {}", r.body);
    let relinked = account();
    assert_eq!(relinked["has_refresh_token"], true, "{relinked}");
    assert_eq!(
        relinked["refresh_token_expires_at"],
        after["refresh_token_expires_at"]
    );
    let access_exp = relinked["access_token_expires_at"]
        .as_u64()
        .unwrap_or_else(|| panic!("access expiry set: {relinked}"));
    assert!(
        access_exp > now() && access_exp <= now() + 600,
        "{relinked}"
    );
}

fn auth_op(
    op: &str,
    provider: Option<&str>,
    token: Option<&str>,
) -> pylon_functions::protocol::AuthOpMessage {
    pylon_functions::protocol::AuthOpMessage {
        call_id: "c".into(),
        op: op.into(),
        provider: provider.map(String::from),
        user_id: None,
        token: token.map(String::from),
    }
}

/// The host side of `ctx.auth.providerToken` refreshing against the live
/// IdP, and `ctx.auth.verifyOidcToken` resolving what it gets back.
#[test]
fn provider_token_refreshes_and_verify_resolves_it() {
    let p = person("fn@example.com");
    let tokens = code_flow(&p, "openid email offline_access");
    let rt = str_of(&tokens, "refresh_token");
    let store = pylon_auth::AccountStore::new();
    let stale = pylon_auth::Account {
        id: "acc-fn".into(),
        user_id: "app-user-1".into(),
        provider_id: "selfidp".into(),
        account_id: p.user_id.clone(),
        access_token: Some(str_of(&tokens, "access_token")),
        refresh_token: Some(rt.clone()),
        id_token: None,
        // Expired: providerToken must refresh before answering.
        access_token_expires_at: Some(now() - 10),
        refresh_token_expires_at: Some(now() + 3600),
        scope: Some("openid email offline_access".into()),
        password: None,
        avatar_url: None,
        external_orgs: None,
        created_at: 1,
        updated_at: 1,
    };
    store.upsert(&stale);

    let out = pylon_runtime::app_auth_ops::handle_op(
        &store,
        &auth_op("provider_token", Some("selfidp"), None),
        Some("app-user-1"),
    )
    .expect("providerToken");
    let fresh = str_of(&out, "accessToken");
    assert_ne!(
        fresh,
        str_of(&tokens, "access_token"),
        "never the stale token"
    );
    assert!(out["expiresAt"].as_u64().unwrap() > now() + 3000);
    // The rotated refresh token and its new expiry were saved.
    let saved = store.find_by_provider("selfidp", &p.user_id).unwrap();
    assert_ne!(saved.refresh_token.as_deref(), Some(rt.as_str()));
    assert!(saved.refresh_token_expires_at.unwrap() > now() + 29 * 24 * 3600);
    assert_eq!(saved.access_token.as_deref(), Some(fresh.as_str()));

    // The IdP side resolves the new token to the IdP user and client.
    let who = pylon_runtime::app_auth_ops::handle_op(
        &store,
        &auth_op("verify_oidc_token", None, Some(&fresh)),
        None,
    )
    .unwrap();
    assert_eq!(who["userId"], p.user_id.as_str());
    assert_eq!(who["clientId"], CLIENT);
    assert!(who["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "offline_access"));
    // A refresh token, or garbage, is not an access token.
    for bad in [saved.refresh_token.as_deref().unwrap(), "garbage"] {
        let out = pylon_runtime::app_auth_ops::handle_op(
            &store,
            &auth_op("verify_oidc_token", None, Some(bad)),
            None,
        )
        .unwrap();
        assert!(out.is_null());
    }

    // No account with the provider: null.
    let out = pylon_runtime::app_auth_ops::handle_op(
        &store,
        &auth_op("provider_token", Some("selfidp"), None),
        Some("someone-else"),
    )
    .unwrap();
    assert!(out.is_null());

    // A refresh token the IdP refuses: REAUTH_REQUIRED, not a stale token.
    let mut dead = stale.clone();
    dead.user_id = "app-user-2".into();
    dead.account_id = "dead-sub".into();
    dead.refresh_token = Some("not-a-real-refresh-token".into());
    store.upsert(&dead);
    let (code, msg) = pylon_runtime::app_auth_ops::handle_op(
        &store,
        &auth_op("provider_token", Some("selfidp"), None),
        Some("app-user-2"),
    )
    .unwrap_err();
    assert_eq!(code, "REAUTH_REQUIRED", "{msg}");
    assert!(!msg.contains(&str_of(&tokens, "access_token")));
}

/// Two threads asking for the same expired account at once: one refresh
/// reaches the IdP, the other waits and gets its result.
#[test]
fn concurrent_refreshes_of_one_account_are_serialized() {
    let p = person("concurrent@example.com");
    let tokens = code_flow(&p, "openid offline_access");
    let store = Arc::new(pylon_auth::AccountStore::new());
    store.upsert(&pylon_auth::Account {
        id: "acc-c".into(),
        user_id: "app-user-c".into(),
        provider_id: "selfidp".into(),
        account_id: p.user_id.clone(),
        access_token: Some("expired".into()),
        refresh_token: Some(str_of(&tokens, "refresh_token")),
        id_token: None,
        access_token_expires_at: Some(now() - 10),
        refresh_token_expires_at: None,
        scope: None,
        password: None,
        avatar_url: None,
        external_orgs: None,
        created_at: 1,
        updated_at: 1,
    });
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let store = Arc::clone(&store);
            let sub = p.user_id.clone();
            std::thread::spawn(move || {
                store
                    .ensure_fresh_access_token("selfidp", &sub, 60)
                    .expect("refresh")
                    .access_token
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(
        results[0], results[1],
        "the second caller reused the first refresh"
    );
    let original = pylon_auth::oidc_tokens::hash_token(&str_of(&tokens, "refresh_token"));
    let row = provider().tokens.find(&original, now()).unwrap().unwrap();
    assert!(row.rotated_at.is_some());
}
