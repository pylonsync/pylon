//! `pylon upgrade` — replace this binary with the latest release.
//!
//! What every other CLI means by "upgrade". The name previously opened
//! a Stripe checkout to move the org to Pro; that lives at
//! `pylon billing upgrade` now.
//!
//! Only the binary installed by scripts/install.sh is replaced in
//! place. A CLI that came from npm or `cargo install` is owned by that
//! package manager — overwriting it would be undone by the next
//! `npm ci` and would leave the manager's metadata lying about what's
//! on disk. Those cases print the command that actually works.
//!
//! The download is checksum-verified against the `.sha256` published
//! beside it, and the replacement only happens after the new binary has
//! been executed once and reported the expected version. There is no
//! flag to skip either check.

use std::io::Read;
use std::path::{Path, PathBuf};

use pylon_kernel::ExitCode;

use crate::output;

const REPO: &str = "pylonsync/pylon";

/// Refuse anything larger than this. The real archive is ~20MB; the cap
/// only exists so a redirect to something unexpected can't be read into
/// memory unbounded.
const MAX_ARCHIVE_BYTES: u64 = 200 * 1024 * 1024;

/// How the running binary got here. Determines whether replacing it is
/// ours to do.
#[derive(Debug, PartialEq, Eq)]
enum InstallKind {
    /// scripts/install.sh, or any plain copy on PATH. Ours to replace.
    Standalone,
    /// A devDependency's bin shim. `npm ci` would undo any edit.
    Npm,
    /// `cargo install pylon-cli`. Cargo tracks the installed version in
    /// .crates2.json; replacing the file behind its back desyncs that.
    Cargo,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return ExitCode::Ok;
    }

    let current = env!("CARGO_PKG_VERSION").to_string();
    let check_only = args.iter().any(|a| a == "--check");
    let pinned = args
        .windows(2)
        .find(|w| w[0] == "--version")
        .map(|w| w[1].trim_start_matches('v').to_string())
        .or_else(|| {
            args.iter().find(|a| a.starts_with("--version=")).map(|a| {
                a.trim_start_matches("--version=")
                    .trim_start_matches('v')
                    .to_string()
            })
        });

    let target_version = match pinned {
        Some(v) => v,
        None => match fetch_latest_version() {
            Ok(v) => v,
            Err(e) => {
                output::print_error(&format!("Couldn't resolve the latest release: {e}"));
                return ExitCode::Unavailable;
            }
        },
    };

    let up_to_date = target_version == current;

    if check_only {
        if json_mode {
            println!(
                "{}",
                serde_json::json!({
                    "current": current,
                    "latest": target_version,
                    "upToDate": up_to_date,
                })
            );
        } else if up_to_date {
            println!("pylon {current} is the latest release.");
        } else {
            println!("pylon {current} → {target_version} available.");
            println!("  Run: pylon upgrade");
        }
        return ExitCode::Ok;
    }

    if up_to_date {
        if json_mode {
            println!(
                "{}",
                serde_json::json!({
                    "ok": true, "current": current, "latest": target_version, "updated": false,
                })
            );
        } else {
            println!("Already on pylon {current}.");
        }
        return ExitCode::Ok;
    }

    let exe = match std::env::current_exe().and_then(|p| p.canonicalize()) {
        Ok(p) => p,
        Err(e) => {
            output::print_error(&format!("Couldn't locate this binary: {e}"));
            return ExitCode::Error;
        }
    };

    match classify_install(&exe) {
        InstallKind::Npm => {
            output::print_error(
                "This pylon came from npm — upgrade it through your package manager.",
            );
            eprintln!("  In your project:  pylon update           # bumps every @pylonsync/* pin");
            eprintln!("  Or directly:      npm i -D @pylonsync/cli@{target_version}");
            ExitCode::Unavailable
        }
        InstallKind::Cargo => {
            output::print_error("This pylon was installed by cargo — upgrade it with cargo.");
            eprintln!("  Run: cargo install pylon-cli --version {target_version} --force");
            ExitCode::Unavailable
        }
        InstallKind::Standalone => {
            match install_release(&exe, &target_version, &current, json_mode) {
                Ok(()) => ExitCode::Ok,
                Err(e) => {
                    output::print_error(&e);
                    ExitCode::Error
                }
            }
        }
    }
}

/// Download, verify, smoke-test, and swap in the release binary.
fn install_release(
    exe: &Path,
    version: &str,
    current: &str,
    json_mode: bool,
) -> Result<(), String> {
    let target = host_target()?;
    let asset = format!("pylon-v{version}-{target}.tar.gz");
    let base = format!("https://github.com/{REPO}/releases/download/v{version}/{asset}");

    if !json_mode {
        eprintln!("→ pylon {current} → {version} ({target})");
    }

    let archive = http_get_bytes(&base).map_err(|e| {
        format!(
            "download failed: {base}\n  {e}\n  \
             If this release just landed, the binaries may still be building — retry shortly."
        )
    })?;

    // The checksum is published beside every asset by the release
    // workflow. A missing one means something is wrong with the release,
    // not that verification is optional — there is no flag to proceed.
    let sums = http_get_bytes(&format!("{base}.sha256")).map_err(|e| {
        format!(
            "no checksum published for {asset} ({e}) — refusing to install an unverified binary"
        )
    })?;
    let expected = String::from_utf8_lossy(&sums)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let actual = sha256_hex(&archive);
    if expected.is_empty() || expected != actual {
        return Err(format!(
            "checksum mismatch for {asset}\n  expected {expected}\n  got      {actual}"
        ));
    }

    let binary = extract_pylon(&archive)?;

    // Stage next to the destination so the final rename is atomic and
    // stays on one filesystem. /tmp is frequently a different mount,
    // where rename fails with EXDEV.
    let dir = exe
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", exe.display()))?;
    let staged = dir.join(format!(".pylon-upgrade-{version}"));
    write_executable(&staged, &binary).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "can't write to {} — this install needs elevated permissions.\n  \
                 Try: sudo pylon upgrade\n  \
                 Or reinstall somewhere you own: \
                 PYLON_INSTALL=$HOME/.pylon curl -fsSL https://www.pylonsync.com/install.sh | bash",
                dir.display()
            )
        } else {
            format!("couldn't stage the new binary in {}: {e}", dir.display())
        }
    })?;

    // Run it before trusting it. A truncated or wrong-arch binary that
    // passed the checksum (i.e. the release itself is broken) would
    // otherwise become the installed pylon and leave no way back.
    if let Err(e) = verify_binary(&staged, version) {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }

    // Renaming over a running executable is fine on Unix: the running
    // process keeps its open inode, and the next invocation gets the
    // new file.
    std::fs::rename(&staged, exe).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("couldn't replace {}: {e}", exe.display())
    })?;

    if json_mode {
        println!(
            "{}",
            serde_json::json!({
                "ok": true, "current": version, "previous": current,
                "updated": true, "path": exe.display().to_string(),
            })
        );
    } else {
        println!("✓ pylon {version} installed at {}", exe.display());
    }
    Ok(())
}

/// Resolve the newest published release from the /releases/latest
/// redirect. No GitHub API call, so no rate limit in CI — same approach
/// as scripts/install.sh.
fn fetch_latest_version() -> Result<String, String> {
    let url = format!("https://github.com/{REPO}/releases/latest");
    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| e.to_string())?;
    let final_url = resp.get_url().to_string();
    parse_tag_from_release_url(&final_url)
        .ok_or_else(|| format!("couldn't read a release tag from {final_url}"))
}

/// `https://github.com/o/r/releases/tag/v0.4.5` → `0.4.5`.
fn parse_tag_from_release_url(url: &str) -> Option<String> {
    let tag = url.trim_end_matches('/').rsplit('/').next()?;
    let version = tag.trim_start_matches('v');
    // Guard against the redirect landing somewhere unexpected (a login
    // wall, an error page) and handing back a garbage "version" that
    // then gets pasted into a download URL.
    let looks_semver = version
        .split('.')
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        && version.split('.').count() == 3;
    looks_semver.then(|| version.to_string())
}

/// The release-asset triple for this host, or an error carrying the
/// same guidance install.sh gives for platforms we don't publish for.
fn host_target() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("macos", arch) => Err(format!(
            "no prebuilt binary for {arch} macOS.\n  \
             Build from source instead: cargo install pylon-cli"
        )),
        ("linux", arch) => Err(format!(
            "no prebuilt binary for {arch} Linux.\n  \
             Run via Docker: ghcr.io/pylonsync/pylon:latest\n  \
             Or build from source: cargo install pylon-cli"
        )),
        (os, _) => Err(format!(
            "no prebuilt binary for {os}.\n  \
             Windows: use WSL2, Docker, or `cargo install pylon-cli`"
        )),
    }
}

/// Which package manager, if any, owns the binary at `exe`.
fn classify_install(exe: &Path) -> InstallKind {
    let path = exe.to_string_lossy();
    if path.contains("/node_modules/") {
        return InstallKind::Npm;
    }
    if is_cargo_path(exe) {
        return InstallKind::Cargo;
    }
    InstallKind::Standalone
}

fn is_cargo_path(exe: &Path) -> bool {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")));
    match cargo_home {
        Some(home) => exe.starts_with(home),
        None => false,
    }
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(120))
        .call()
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(MAX_ARCHIVE_BYTES)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Pull the `pylon` entry out of the release tarball.
fn extract_pylon(archive: &[u8]) -> Result<Vec<u8>, String> {
    let decoder = flate2::read::GzDecoder::new(archive);
    let mut tar = tar::Archive::new(decoder);
    let entries = tar
        .entries()
        .map_err(|e| format!("couldn't read the release archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("couldn't read the release archive: {e}"))?;
        let is_pylon = entry
            .path()
            .map(|p| p.file_name().map(|n| n == "pylon").unwrap_or(false))
            .unwrap_or(false);
        if !is_pylon {
            continue;
        }
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("couldn't extract the pylon binary: {e}"))?;
        return Ok(buf);
    }
    Err("the release archive did not contain a 'pylon' binary".into())
}

fn write_executable(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Run the staged binary and confirm it reports the version we asked
/// for. Catches a wrong-arch or corrupt build before it replaces a
/// working install.
fn verify_binary(staged: &Path, expected_version: &str) -> Result<(), String> {
    let out = std::process::Command::new(staged)
        .arg("version")
        .output()
        .map_err(|e| format!("the downloaded binary wouldn't run: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "the downloaded binary exited with {} on `pylon version`",
            out.status
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !stdout.contains(expected_version) {
        return Err(format!(
            "the downloaded binary reports \"{}\", expected {expected_version}",
            stdout.trim()
        ));
    }
    Ok(())
}

fn print_help() {
    println!("pylon upgrade — update this binary to the latest release");
    println!();
    println!("USAGE");
    println!("  pylon upgrade [--check] [--version <v>] [--json]");
    println!();
    println!("Replaces the pylon installed by install.sh. A CLI installed by npm or");
    println!("cargo is left alone — those print the command that upgrades it properly.");
    println!();
    println!("The archive is verified against its published SHA-256 and executed once");
    println!("before it replaces anything. Neither check can be skipped.");
    println!();
    println!("FLAGS");
    println!("  --check          Report the current and latest versions, change nothing");
    println!("  --version <v>    Install a specific release instead of the latest");
    println!("  --json           Machine-readable result");
    println!("  -h, --help       Show this help");
    println!();
    println!("To move your organization to the Pro plan, use `pylon billing upgrade`.");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_tag_off_a_release_redirect() {
        assert_eq!(
            parse_tag_from_release_url("https://github.com/pylonsync/pylon/releases/tag/v0.4.5")
                .as_deref(),
            Some("0.4.5")
        );
    }

    #[test]
    fn tolerates_a_trailing_slash() {
        assert_eq!(
            parse_tag_from_release_url("https://github.com/pylonsync/pylon/releases/tag/v1.2.30/")
                .as_deref(),
            Some("1.2.30")
        );
    }

    #[test]
    fn rejects_a_redirect_that_is_not_a_release() {
        // A login wall or error page must not yield a "version" that
        // then gets pasted into a download URL.
        assert_eq!(
            parse_tag_from_release_url("https://github.com/login?return_to=%2Fpylonsync"),
            None
        );
        assert_eq!(
            parse_tag_from_release_url("https://github.com/pylonsync/pylon/releases"),
            None
        );
        assert_eq!(
            parse_tag_from_release_url("https://github.com/pylonsync/pylon/releases/tag/nightly"),
            None
        );
    }

    #[test]
    fn npm_install_is_not_ours_to_replace() {
        assert_eq!(
            classify_install(Path::new("/app/node_modules/.bin/pylon")),
            InstallKind::Npm
        );
    }

    #[test]
    fn a_plain_bin_dir_is_standalone() {
        assert_eq!(
            classify_install(Path::new("/usr/local/bin/pylon")),
            InstallKind::Standalone
        );
        assert_eq!(
            classify_install(Path::new("/home/me/.pylon/bin/pylon")),
            InstallKind::Standalone
        );
    }

    #[test]
    fn cargo_home_is_detected_from_the_env() {
        // Scoped to this assertion rather than $HOME so the test doesn't
        // depend on where it runs.
        std::env::set_var("CARGO_HOME", "/opt/cargo");
        assert!(is_cargo_path(Path::new("/opt/cargo/bin/pylon")));
        assert!(!is_cargo_path(Path::new("/usr/local/bin/pylon")));
        std::env::remove_var("CARGO_HOME");
    }

    #[test]
    fn sha256_matches_a_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn extracts_the_pylon_entry_from_a_tarball() {
        use flate2::write::GzEncoder;
        use std::io::Write;

        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let payload = b"#!/bin/sh\necho pylon\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "pylon", &payload[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let mut gz = GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar_buf).unwrap();
        let archive = gz.finish().unwrap();

        assert_eq!(extract_pylon(&archive).unwrap(), b"#!/bin/sh\necho pylon\n");
    }

    #[test]
    fn a_tarball_without_pylon_is_an_error() {
        use flate2::write::GzEncoder;
        use std::io::Write;

        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let payload = b"nope";
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "README", &payload[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let mut gz = GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar_buf).unwrap();
        let archive = gz.finish().unwrap();

        assert!(extract_pylon(&archive).is_err());
    }
}
