//! Installable, offline-capable apps (`buildManifest({ offline })`).
//!
//! Three framework URLs:
//!
//! - `/manifest.webmanifest`: the web app manifest. An app's own file
//!   (`public/manifest.webmanifest`, then `<appDir>/manifest.webmanifest`)
//!   wins; otherwise it is generated from the `offline` config.
//! - `/pylon-sw.js`: the service worker. The client bundler writes it into
//!   the client build next to `manifest.json`, with the build's precache list
//!   inlined (see `packages/functions/src/offline-sw.ts`). Served from the
//!   site root so its scope covers the whole app without a
//!   `Service-Worker-Allowed` header, and with `no-cache` so the browser
//!   checks for a new build on every launch.
//! - `/_pylon/version`: the build's version, build time and install identity
//!   (`pylon-version.json` in the client build, written next to the worker).
//!   `useAppUpdate` in `@pylonsync/react` polls it while the app is open, so
//!   it is served with `no-store`.
//!
//! The SSR side marks a page render requested by the service worker
//! (`X-Pylon-Offline-Shell: 1`) as an offline shell only when its HTML does
//! not depend on who is signed in; see [`OFFLINE_SHELL_RESPONSE_HEADER`].

use super::{content_type_for, open_static_file, respond_static_reader};
use std::path::{Path, PathBuf};
use tiny_http::{Header, Request, Response};

/// Request header the service worker sends when it fetches a route's HTML
/// to keep for offline use.
pub(crate) const OFFLINE_SHELL_REQUEST_HEADER: &str = "x-pylon-offline-shell";

/// Response header the host sets on an offline-shell render: `1` when the
/// HTML is the same for every signed-in user (the service worker stores it),
/// or `refused; <reason>` (the service worker does not).
pub(crate) const OFFLINE_SHELL_RESPONSE_HEADER: &str = "Pylon-Offline-Shell";

/// The service worker file inside the client build.
pub(crate) const SERVICE_WORKER_FILE: &str = "pylon-sw.js";

/// The build version file inside the client build, served at
/// [`BUILD_VERSION_PATH`].
pub(crate) const BUILD_VERSION_FILE: &str = "pylon-version.json";

/// Where the runtime serves [`BUILD_VERSION_FILE`].
pub(crate) const BUILD_VERSION_PATH: &str = "/_pylon/version";

/// Where `/manifest.webmanifest` comes from.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WebManifestSource {
    /// The app ships its own file.
    File(PathBuf),
    /// Generated from `buildManifest({ offline })`.
    Generated(Vec<u8>),
}

/// Resolve `/manifest.webmanifest`. `None` when the app has neither a file
/// nor an `offline` config: the request falls through to the 404.
pub(crate) fn web_manifest_source(
    offline: Option<&pylon_kernel::ManifestOffline>,
    public_dir: &Path,
    app_dir: &Path,
) -> Option<WebManifestSource> {
    for candidate in [
        public_dir.join("manifest.webmanifest"),
        app_dir.join("manifest.webmanifest"),
    ] {
        if candidate.is_file() {
            return Some(WebManifestSource::File(candidate));
        }
    }
    let offline = offline?;
    let body = serde_json::to_vec_pretty(&offline.web_manifest()).ok()?;
    Some(WebManifestSource::Generated(body))
}

const WEB_MANIFEST_CONTENT_TYPE: &str = "application/manifest+json";

/// Serve `/manifest.webmanifest`, or hand the request back when there is
/// nothing to serve.
pub(crate) fn serve_web_manifest(
    request: Request,
    offline: Option<&pylon_kernel::ManifestOffline>,
    public_dir: &Path,
    app_dir: &Path,
    cache: &str,
    cors_origin: &str,
) -> Result<(), Request> {
    match web_manifest_source(offline, public_dir, app_dir) {
        Some(WebManifestSource::File(path)) => match open_static_file(&path) {
            Ok((file, length)) => {
                respond_static_reader(
                    request,
                    file,
                    length,
                    WEB_MANIFEST_CONTENT_TYPE,
                    cache,
                    cors_origin,
                );
                Ok(())
            }
            Err(_) => Err(request),
        },
        Some(WebManifestSource::Generated(body)) => {
            let length = body.len();
            respond_static_reader(
                request,
                std::io::Cursor::new(body),
                length,
                WEB_MANIFEST_CONTENT_TYPE,
                cache,
                cors_origin,
            );
            Ok(())
        }
        None => Err(request),
    }
}

/// Serve `/pylon-sw.js` from the client build at `outdir`. A missing file
/// (an app built before it opted in, or a build that failed) answers 404,
/// which makes the browser drop the registration on its next update check.
pub(crate) fn serve_service_worker(request: Request, outdir: Option<&Path>, cors_origin: &str) {
    let file = outdir
        .map(|dir| dir.join(SERVICE_WORKER_FILE))
        .and_then(|path| open_static_file(&path).ok());
    match file {
        Some((file, length)) => {
            // `no-cache`: the browser revalidates on every update check, so a
            // deploy reaches installed apps on their next launch.
            respond_static_reader(
                request,
                file,
                length,
                content_type_for(Path::new(SERVICE_WORKER_FILE)),
                "no-cache",
                cors_origin,
            );
        }
        None => {
            let mut response =
                Response::from_data(b"// no service worker in this build\n".to_vec())
                    .with_status_code(404u16);
            if let Ok(h) =
                Header::from_bytes("Content-Type", "application/javascript; charset=utf-8")
            {
                response = response.with_header(h);
            }
            if let Ok(h) = Header::from_bytes("Cache-Control", "no-store") {
                response = response.with_header(h);
            }
            let _ = request.respond(response);
        }
    }
}

/// Serve `/_pylon/version` from the client build at `outdir`, with
/// `no-store`: a cached answer would hide a deploy from an app that stays
/// open. A missing file (a build that failed, or one made before the app
/// opted in) answers 404.
pub(crate) fn serve_build_version(request: Request, outdir: Option<&Path>, cors_origin: &str) {
    let file = outdir
        .map(|dir| dir.join(BUILD_VERSION_FILE))
        .and_then(|path| open_static_file(&path).ok());
    match file {
        Some((file, length)) => respond_static_reader(
            request,
            file,
            length,
            "application/json",
            "no-store",
            cors_origin,
        ),
        None => {
            let mut response = Response::from_data(b"{\"error\":\"no build version\"}".to_vec())
                .with_status_code(404u16);
            for (name, value) in [
                ("Content-Type", "application/json"),
                ("Cache-Control", "no-store"),
            ] {
                if let Ok(h) = Header::from_bytes(name, value) {
                    response = response.with_header(h);
                }
            }
            let _ = request.respond(response);
        }
    }
}

/// Whether the request asks for an offline shell (sent by the service worker
/// when it stores a route's HTML).
pub(crate) fn is_offline_shell_request(request: &Request) -> bool {
    request.headers().iter().any(|h| {
        h.field
            .as_str()
            .as_str()
            .eq_ignore_ascii_case(OFFLINE_SHELL_REQUEST_HEADER)
            && h.value.as_str().trim() == "1"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::{ManifestOffline, ManifestOfflineIcon};

    fn config() -> ManifestOffline {
        ManifestOffline {
            name: "Field Sales".into(),
            short_name: Some("Sales".into()),
            description: None,
            icons: vec![ManifestOfflineIcon {
                src: "/icons/192.png".into(),
                sizes: "192x192".into(),
                mime_type: Some("image/png".into()),
                purpose: None,
            }],
            theme_color: Some("#0b2545".into()),
            background_color: Some("#ffffff".into()),
            start_url: "/leads".into(),
            display: "standalone".into(),
            routes: vec!["/leads".into(), "/leads/[id]".into()],
        }
    }

    /// Run one GET through `try_handle` and return (status, headers, body).
    fn get_via_try_handle(cfg: super::super::FrontendConfig, path: &str) -> (u16, String, Vec<u8>) {
        use std::io::{Read, Write};
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        let worker = std::thread::spawn(move || {
            let request = server
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .unwrap();
            if let Err(request) = super::super::try_handle(&cfg, request, "*") {
                // What the API router answers for an unknown path.
                let _ = request.respond(Response::empty(404));
            }
        });
        let mut client = std::net::TcpStream::connect(addr).unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        write!(
            client,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut wire = Vec::new();
        client.read_to_end(&mut wire).unwrap();
        worker.join().unwrap();
        let end = wire.windows(4).position(|b| b == b"\r\n\r\n").unwrap();
        let head = std::str::from_utf8(&wire[..end])
            .unwrap()
            .to_ascii_lowercase();
        let status: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, head, wire[end + 4..].to_vec())
    }

    fn frontend(offline: Option<ManifestOffline>) -> super::super::FrontendConfig {
        super::super::FrontendConfig::from_env(Path::new("/nonexistent-app-dir"))
            .with_offline(offline.map(std::sync::Arc::new))
    }

    #[test]
    fn a_configured_app_serves_a_generated_web_manifest() {
        let (status, head, body) =
            get_via_try_handle(frontend(Some(config())), "/manifest.webmanifest");
        assert_eq!(status, 200, "{head}");
        assert!(
            head.contains("content-type: application/manifest+json"),
            "{head}"
        );
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "Field Sales");
        assert_eq!(json["short_name"], "Sales");
        assert_eq!(json["start_url"], "/leads");
        assert_eq!(json["scope"], "/");
        assert_eq!(json["display"], "standalone");
        assert_eq!(json["theme_color"], "#0b2545");
        assert_eq!(json["background_color"], "#ffffff");
        assert_eq!(json["icons"][0]["src"], "/icons/192.png");
        assert_eq!(json["icons"][0]["sizes"], "192x192");
        assert_eq!(json["icons"][0]["type"], "image/png");
    }

    #[test]
    fn an_unconfigured_app_has_no_web_manifest_or_service_worker() {
        let (status, _, _) = get_via_try_handle(frontend(None), "/manifest.webmanifest");
        assert_eq!(status, 404);
        let (status, _, _) = get_via_try_handle(frontend(None), "/pylon-sw.js");
        assert_eq!(status, 404);
        let (status, _, _) = get_via_try_handle(frontend(None), BUILD_VERSION_PATH);
        assert_eq!(status, 404);
    }

    #[test]
    fn a_configured_app_without_a_build_answers_the_version_with_404_no_store() {
        // No client build (no function runtime): the version endpoint is
        // claimed by the offline app, but there is no file to serve.
        let (status, head, body) =
            get_via_try_handle(frontend(Some(config())), "/_pylon/version?t=1");
        assert_eq!(status, 404, "{head}");
        assert!(head.contains("cache-control: no-store"), "{head}");
        assert!(head.contains("content-type: application/json"), "{head}");
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "no build version");
    }

    #[test]
    fn the_build_version_is_served_from_the_build_without_storing() {
        use std::io::{Read, Write};
        let outdir = tempfile::tempdir().unwrap();
        let file = br#"{"version":"0123456789abcdef","builtAt":"2026-10-22T10:00:00.000Z","install":"fedcba9876543210"}"#;
        std::fs::write(outdir.path().join(BUILD_VERSION_FILE), file).unwrap();
        let outdir_path = outdir.path().to_path_buf();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        let worker = std::thread::spawn(move || {
            for dir in [Some(outdir_path), None] {
                let request = server
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                serve_build_version(request, dir.as_deref(), "*");
            }
        });
        let fetch = || {
            let mut client = std::net::TcpStream::connect(addr).unwrap();
            client
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            write!(
                client,
                "GET /_pylon/version HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut wire = Vec::new();
            client.read_to_end(&mut wire).unwrap();
            let end = wire.windows(4).position(|b| b == b"\r\n\r\n").unwrap();
            (
                std::str::from_utf8(&wire[..end])
                    .unwrap()
                    .to_ascii_lowercase(),
                wire[end + 4..].to_vec(),
            )
        };
        let (head, body) = fetch();
        assert!(head.starts_with("http/1.1 200"), "{head}");
        assert!(head.contains("content-type: application/json"), "{head}");
        assert!(head.contains("cache-control: no-store"), "{head}");
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["version"], "0123456789abcdef");
        assert_eq!(json["builtAt"], "2026-10-22T10:00:00.000Z");
        assert_eq!(json["install"], "fedcba9876543210");
        let (head, _) = fetch();
        assert!(head.starts_with("http/1.1 404"), "{head}");
        assert!(head.contains("cache-control: no-store"), "{head}");
        worker.join().unwrap();
    }

    #[test]
    fn the_apps_own_manifest_file_wins() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        let app = dir.path().join("app");
        std::fs::create_dir_all(&public).unwrap();
        std::fs::create_dir_all(&app).unwrap();
        let cfg = config();

        // No file: generated.
        assert!(matches!(
            web_manifest_source(Some(&cfg), &public, &app),
            Some(WebManifestSource::Generated(_))
        ));
        // app/manifest.webmanifest beats the config.
        std::fs::write(app.join("manifest.webmanifest"), b"{\"name\":\"app\"}").unwrap();
        assert_eq!(
            web_manifest_source(Some(&cfg), &public, &app),
            Some(WebManifestSource::File(app.join("manifest.webmanifest")))
        );
        // public/manifest.webmanifest beats both, and works with no config.
        std::fs::write(public.join("manifest.webmanifest"), b"{\"name\":\"pub\"}").unwrap();
        assert_eq!(
            web_manifest_source(None, &public, &app),
            Some(WebManifestSource::File(public.join("manifest.webmanifest")))
        );
        // Neither: nothing.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(web_manifest_source(None, empty.path(), empty.path()), None);
    }

    #[test]
    fn the_service_worker_is_served_from_the_build_without_caching() {
        use std::io::{Read, Write};
        let outdir = tempfile::tempdir().unwrap();
        std::fs::write(outdir.path().join(SERVICE_WORKER_FILE), b"self.x = 1;\n").unwrap();
        let outdir_path = outdir.path().to_path_buf();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        let worker = std::thread::spawn(move || {
            for dir in [Some(outdir_path), None] {
                let request = server
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                serve_service_worker(request, dir.as_deref(), "*");
            }
        });
        let fetch = || {
            let mut client = std::net::TcpStream::connect(addr).unwrap();
            client
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            write!(
                client,
                "GET /pylon-sw.js HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut wire = Vec::new();
            client.read_to_end(&mut wire).unwrap();
            let end = wire.windows(4).position(|b| b == b"\r\n\r\n").unwrap();
            (
                std::str::from_utf8(&wire[..end])
                    .unwrap()
                    .to_ascii_lowercase(),
                wire[end + 4..].to_vec(),
            )
        };
        let (head, body) = fetch();
        assert!(head.starts_with("http/1.1 200"), "{head}");
        assert!(
            head.contains("content-type: application/javascript"),
            "{head}"
        );
        assert!(head.contains("cache-control: no-cache"), "{head}");
        assert_eq!(body, b"self.x = 1;\n");
        let (head, _) = fetch();
        assert!(head.starts_with("http/1.1 404"), "{head}");
        worker.join().unwrap();
    }
}
