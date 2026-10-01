//! The C# client's Apple, Google, and Steam sign-in against the real routes.
//!
//! Starts a server whose Apple and Google JWKS caches hold a key this test
//! controls, and whose Steam Web API is the fake in tests/support, then
//! runs `LiveNativeSignInTests` in packages/csharp with the server URL and
//! signed tokens in the environment.
//!
//! Needs the .NET SDK: `dotnet` on PATH, or `DOTNET` naming the binary.
//! Skipped with a message without it, unless `PYLON_REQUIRE_DOTNET` is set
//! (CI sets it), which makes a missing SDK a failure.

#[path = "support/fake_steam.rs"]
mod fake_steam;

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use pylon_auth::native_id_token::{seed_jwks_cache, NativeProvider};
use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, ManifestPolicy};
use pylon_runtime::Runtime;
use rsa::traits::PublicKeyParts;
use rsa::{Pkcs1v15Sign, RsaPrivateKey};
use sha2::{Digest, Sha256};

const BUNDLE_ID: &str = "com.example.camelot";
const GOOGLE_CLIENT_ID: &str = "camelot.apps.googleusercontent.com";
const APP_ID: &str = "480";
const IDENTITY: &str = "camelot-pylon";

fn field(name: &str, optional: bool, unique: bool) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional,
        unique,
        ..Default::default()
    }
}

fn manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "native-signin-csharp".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "User".into(),
            fields: vec![
                field("email", false, true),
                field("displayName", true, false),
                field("emailVerified", true, false),
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

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn dotnet() -> Option<String> {
    let bin = std::env::var("DOTNET").unwrap_or_else(|_| "dotnet".into());
    Command::new(&bin)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| bin)
}

struct Signer {
    key: RsaPrivateKey,
}

impl Signer {
    fn publish(&self) {
        let public = self.key.to_public_key();
        let jwks = serde_json::json!({ "keys": [{
            "kty": "RSA", "kid": "csharp-key", "alg": "RS256", "use": "sig",
            "n": URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
            "e": URL_SAFE_NO_PAD.encode(public.e().to_bytes_be()),
        }]})
        .to_string();
        seed_jwks_cache(NativeProvider::Apple.jwks_url(), &jwks);
        seed_jwks_cache(NativeProvider::Google.jwks_url(), &jwks);
    }

    fn token(&self, iss: &str, aud: &str, sub: &str, email: &str) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","kid":"csharp-key"}"#);
        let claims = serde_json::json!({
            "iss": iss, "aud": aud, "exp": now + 600, "iat": now,
            "sub": sub, "email": email, "email_verified": true,
        });
        let input = format!("{header}.{}", URL_SAFE_NO_PAD.encode(claims.to_string()));
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
fn the_csharp_client_signs_in_with_apple_google_and_steam() {
    let Some(dotnet) = dotnet() else {
        if std::env::var("PYLON_REQUIRE_DOTNET").is_ok() {
            panic!("PYLON_REQUIRE_DOTNET is set and the .NET SDK is missing");
        }
        eprintln!("skipped: the .NET SDK is not installed");
        return;
    };

    let steam = fake_steam::start();
    // SAFETY: set before the server thread starts; this binary has one test.
    unsafe {
        std::env::set_var("PYLON_DEV_MODE", "1");
        std::env::set_var("PYLON_APPLE_NATIVE_CLIENT_IDS", BUNDLE_ID);
        std::env::set_var("PYLON_GOOGLE_NATIVE_CLIENT_IDS", GOOGLE_CLIENT_ID);
        std::env::set_var("PYLON_STEAM_WEB_API_KEY", "test-steam-key");
        std::env::set_var("PYLON_STEAM_APP_ID", APP_ID);
        std::env::set_var("PYLON_STEAM_IDENTITY", IDENTITY);
        std::env::set_var("PYLON_STEAM_API_BASE", &steam);
        std::env::set_var("PYLON_AUTH_VERIFY_IP_PER_MIN", "10000");
    }
    let signer = Signer {
        key: RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap(),
    };
    signer.publish();

    let port = {
        let mut port = 0;
        for _ in 0..200 {
            let base = 25_200 + rand::random::<u16>() % 6_000;
            if (0..4).all(|o| pylon_runtime::listen::port_is_free(base + o)) {
                port = base;
                break;
            }
        }
        assert_ne!(port, 0, "no free port block");
        port
    };
    let rt = Arc::new(Runtime::in_memory(manifest()).unwrap());
    std::thread::spawn(move || {
        let _ = pylon_runtime::server::start(rt, port);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(std::time::Instant::now() < deadline, "server never bound");
        std::thread::sleep(Duration::from_millis(50));
    }

    let apple = signer.token(
        "https://appleid.apple.com",
        BUNDLE_ID,
        "001234.camelot",
        "jane@example.com",
    );
    let google = signer.token(
        "https://accounts.google.com",
        GOOGLE_CLIENT_ID,
        "1078",
        "sam@example.com",
    );
    let output = Command::new(&dotnet)
        .current_dir(repo_root().join("packages/csharp"))
        .args([
            "test",
            "Tests~/Pylon.Tests",
            "--filter",
            "FullyQualifiedName~LiveNativeSignInTests",
            "--logger",
            "console;verbosity=normal",
        ])
        .env(
            "PYLON_NATIVE_SIGNIN_URL",
            format!("http://127.0.0.1:{port}"),
        )
        .env("PYLON_TEST_APPLE_ID_TOKEN", apple)
        .env("PYLON_TEST_GOOGLE_ID_TOKEN", google)
        .env(
            "PYLON_TEST_STEAM_TICKET",
            fake_steam::ticket("76561197960287930", APP_ID, IDENTITY, false),
        )
        .env(
            "PYLON_TEST_STEAM_FOREIGN_TICKET",
            fake_steam::ticket("76561197960287930", APP_ID, "another-service", false),
        )
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .output()
        .expect("run dotnet test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "dotnet test failed:\n{stdout}\n{stderr}"
    );
    // All four live tests ran and passed; none were skipped.
    let passed = stdout
        .lines()
        .filter(|l| {
            l.trim_start()
                .starts_with("Passed Pylon.Tests.LiveNativeSignInTests.")
        })
        .count();
    let skipped = stdout
        .lines()
        .filter(|l| {
            l.trim_start()
                .starts_with("Skipped Pylon.Tests.LiveNativeSignInTests.")
        })
        .count();
    assert_eq!(
        (passed, skipped),
        (4, 0),
        "expected 4 passed, 0 skipped:\n{stdout}"
    );
}
