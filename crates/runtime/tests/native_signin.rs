//! `POST /api/auth/native/<provider>` — sign in with a platform id_token.
//!
//! Signs a Sign-in-with-Apple-shaped token with a locally generated RSA key,
//! seeds the JWKS cache with the matching public key (so no network fetch
//! happens), and drives the real HTTP route:
//!
//!   - a valid token mints a session whose bearer resolves on /api/auth/me
//!   - a second sign-in with the same `sub` lands on the same user
//!   - a token for another app's bundle id is refused
//!   - an unknown provider is a 404, a missing id_token a 400

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use pylon_auth::native_id_token::{seed_jwks_cache, NativeProvider};
use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, ManifestPolicy};
use pylon_runtime::Runtime;
use rsa::traits::PublicKeyParts;
use rsa::{Pkcs1v15Sign, RsaPrivateKey};
use sha2::{Digest, Sha256};

const BUNDLE_ID: &str = "com.example.nativeapp";

fn field(name: &str, optional: bool, unique: bool) -> ManifestField {
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
    }
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "native-signin".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "User".into(),
            fields: vec![
                field("email", false, true),
                field("displayName", true, false),
                field("emailVerified", true, false),
                field("avatarColor", true, false),
                field("createdAt", true, false),
            ],
            ..Default::default()
        }],
        policies: vec![ManifestPolicy {
            name: "user_self".into(),
            entity: Some("User".into()),
            allow_read: Some("auth.userId == data.id".into()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn available_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(24_400);
    for _ in 0..200 {
        let base = NEXT.fetch_add(4, Ordering::Relaxed);
        let ok = (0..4)
            .all(|off| std::net::TcpListener::bind(format!("127.0.0.1:{}", base + off)).is_ok());
        if ok {
            return base;
        }
    }
    panic!("no free 4-port block");
}

fn start_server() -> u16 {
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_APPLE_NATIVE_CLIENT_IDS", BUNDLE_ID);
    });
    let port = available_port();
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt, port);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return port;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server never bound port {port}");
}

fn http(port: u16, method: &str, path: &str, bearer: Option<&str>, body: &str) -> (u16, String) {
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(t) = bearer {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    stream.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    let status: u16 = raw
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

struct AppleSigner {
    key: RsaPrivateKey,
}

impl AppleSigner {
    fn new() -> Self {
        Self {
            key: RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap(),
        }
    }

    fn seed_jwks(&self) {
        let public = self.key.to_public_key();
        let jwks = serde_json::json!({
            "keys": [{
                "kty": "RSA", "kid": "test-key", "alg": "RS256", "use": "sig",
                "n": URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
                "e": URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
            }]
        });
        seed_jwks_cache(NativeProvider::Apple.jwks_url(), &jwks.to_string());
    }

    fn token(&self, sub: &str, email: &str, aud: &str) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","kid":"test-key"}"#);
        let claims = serde_json::json!({
            "iss": "https://appleid.apple.com",
            "aud": aud,
            "exp": now + 600,
            "iat": now,
            "sub": sub,
            "email": email,
            "email_verified": "true",
        });
        let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
        let input = format!("{header}.{payload}");
        let sig = self
            .key
            .sign(
                Pkcs1v15Sign::new::<Sha256>(),
                &Sha256::digest(input.as_bytes()),
            )
            .unwrap();
        format!("{input}.{}", URL_SAFE_NO_PAD.encode(sig))
    }
}

#[test]
fn native_apple_sign_in_mints_a_session_and_links_the_account() {
    let signer = AppleSigner::new();
    signer.seed_jwks();
    let port = start_server();

    let token = signer.token("001234.abcdef", "Jane@Example.com", BUNDLE_ID);
    let (status, body) = http(
        port,
        "POST",
        "/api/auth/native/apple",
        None,
        &serde_json::json!({ "id_token": token, "name": "Jane Doe" }).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let session = v["token"].as_str().unwrap().to_string();
    let user_id = v["user_id"].as_str().unwrap().to_string();
    assert_eq!(v["provider"], "apple");

    // The bearer resolves to the new user.
    let (status, me) = http(port, "GET", "/api/auth/me", Some(&session), "");
    assert_eq!(status, 200, "{me}");
    let me: serde_json::Value = serde_json::from_str(&me).unwrap();
    assert_eq!(me["user_id"], user_id);

    // The row exists with the canonical (lowercased) email and the name
    // the app forwarded.
    let (status, row) = http(
        port,
        "GET",
        &format!("/api/entities/User/{user_id}"),
        Some(&session),
        "",
    );
    assert_eq!(status, 200, "{row}");
    let row: serde_json::Value = serde_json::from_str(&row).unwrap();
    assert_eq!(row["email"], "jane@example.com");
    assert_eq!(row["displayName"], "Jane Doe");

    // Same Apple subject again: the existing account, not a second row.
    let again = signer.token("001234.abcdef", "jane@example.com", BUNDLE_ID);
    let (status, body) = http(
        port,
        "POST",
        "/api/auth/native/apple",
        None,
        &serde_json::json!({ "id_token": again }).to_string(),
    );
    assert_eq!(status, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["user_id"], user_id,
        "second sign-in reuses the linked user"
    );
}

#[test]
fn native_sign_in_rejects_other_apps_tokens_and_bad_requests() {
    let signer = AppleSigner::new();
    signer.seed_jwks();
    let port = start_server();

    // A token minted for a different bundle id.
    let foreign = signer.token("999.zzz", "mallory@example.com", "com.other.app");
    let (status, body) = http(
        port,
        "POST",
        "/api/auth/native/apple",
        None,
        &serde_json::json!({ "id_token": foreign }).to_string(),
    );
    assert_eq!(status, 401, "{body}");
    assert!(body.contains("INVALID_ID_TOKEN"));

    // Unknown provider.
    let (status, _) = http(port, "POST", "/api/auth/native/facebook", None, "{}");
    assert_eq!(status, 404);

    // Google has no audience configured on this server.
    let (status, body) = http(
        port,
        "POST",
        "/api/auth/native/google",
        None,
        &serde_json::json!({ "id_token": "x.y.z" }).to_string(),
    );
    assert_eq!(status, 404, "{body}");

    // Missing id_token.
    let (status, body) = http(port, "POST", "/api/auth/native/apple", None, "{}");
    assert_eq!(status, 400, "{body}");
}
