//! Prebuilt-artifact bootstrap for `pylon start` (Pylon Cloud build pipeline).
//!
//! In the build-artifact deploy model (see pylon-cloud
//! `docs/rfc-deploy-build-pipeline.md`), a deploy ships a single **prebuilt
//! bundle** — app code + `node_modules` + built `web/dist` + `public/` —
//! rather than inlining source into the Fly machine config (which is capped
//! ~950 KB and silently dropped `public/`). The control plane builds the
//! bundle on an ephemeral builder, uploads it to object storage (Tigris), and
//! injects `PYLON_ARTIFACT_*` env into the app machine. On boot, before any
//! `app.ts` is read, this module fetches + verifies + extracts that bundle.
//!
//! Design notes:
//!   - **Backward compatible.** Unset `PYLON_ARTIFACT_ID` → no-op → the legacy
//!     inline-files boot is unchanged. Nothing happens until the control plane
//!     opts a deploy in.
//!   - **Fail closed on integrity.** When an artifact IS requested
//!     (`PYLON_ARTIFACT_ID` set) a valid 64-hex `PYLON_ARTIFACT_SHA256` is
//!     REQUIRED, and the downloaded bytes are verified against it before
//!     extraction. A missing/garbage hash aborts boot — never extract an
//!     unverified bundle.
//!   - **Bounded.** Download + decompressed size are capped so a malicious or
//!     runaway bundle can't fill the shared `/data` volume and take the app
//!     (DB, uploads) down with it.
//!   - **Atomic + per-id.** The bundle extracts to `<base>/<id>/` on the
//!     persistent `/data` volume and the runtime chdirs into it. The DB +
//!     uploads live elsewhere on `/data` (absolute paths) and are untouched.
//!   - **Cache + last-good.** A `<base>/<id>/.pylon-artifact-complete` marker
//!     means "already extracted" → skip the fetch on restart (no re-download,
//!     no multi-region egress). If a fetch fails but a previously-extracted
//!     artifact exists, boot that (last-good) so a Tigris blip never takes a
//!     running app down. Fail closed only when there is nothing cached.

use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Marker file written (last) inside a fully-extracted artifact dir.
const COMPLETE_MARKER: &str = ".pylon-artifact-complete";
/// How many PRIOR extracted artifacts to keep for rollback (the current one is
/// always kept on top of this). Deliberately small: the `/data` volume is sized
/// for roughly current + this many bundles, and each prebuilt bundle (with
/// node_modules) is large — keeping too many is exactly what let stale
/// artifacts fill the volume and wedge boots (unpack-fails-on-full-disk).
const KEEP_ARTIFACTS: usize = 1;
/// Default extraction base on Pylon Cloud (the persistent Fly volume).
const DEFAULT_BASE: &str = "/data/.pylon-artifact";
/// Hard cap on the downloaded bundle (compressed). Full-prebuilt bundles with
/// `node_modules` are large but bounded; this stops a runaway/hostile object
/// from filling the volume.
const MAX_DOWNLOAD_BYTES: u64 = 3 * 1024 * 1024 * 1024;
/// Hard cap on total decompressed bytes (decompression-bomb guard).
const MAX_EXTRACT_BYTES: u64 = 6 * 1024 * 1024 * 1024;
/// Cap on the token-mint response body.
const MAX_TOKEN_BODY_BYTES: u64 = 64 * 1024;

/// Resolved artifact configuration, parsed from `PYLON_ARTIFACT_*` env.
struct Cfg {
    id: String,
    sha256: String,
    /// Pre-signed GET url to the `.tar.gz`. Empty if `token_url` is used.
    url: String,
    /// Optional control-plane endpoint that mints a fresh signed url (GET →
    /// `{"url": "..."}`). Preferred when set — keeps a standing signed url out
    /// of the machine env.
    token_url: String,
    base: PathBuf,
}

/// Read `PYLON_ARTIFACT_*` from the environment. Returns `None` (legacy boot)
/// unless `PYLON_ARTIFACT_ID` is set.
fn cfg_from_env() -> Option<Cfg> {
    let id = non_empty(std::env::var("PYLON_ARTIFACT_ID").ok())?;
    Some(Cfg {
        id,
        sha256: std::env::var("PYLON_ARTIFACT_SHA256")
            .unwrap_or_default()
            .trim()
            .to_lowercase(),
        url: std::env::var("PYLON_ARTIFACT_URL").unwrap_or_default(),
        token_url: std::env::var("PYLON_ARTIFACT_TOKEN_URL").unwrap_or_default(),
        base: std::env::var("PYLON_ARTIFACT_DIR")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_BASE)),
    })
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.filter(|s| !s.trim().is_empty())
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Removes a file on drop — guarantees the multi-hundred-MB download temp is
/// cleaned up on every error/early-return path, not just the happy one.
struct ScopedRemove(PathBuf);
impl Drop for ScopedRemove {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
struct ScopedRemoveDir(PathBuf);
impl Drop for ScopedRemoveDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Ensure the prebuilt bundle for this deploy is present on disk and return
/// the directory the runtime should treat as the app root (to `chdir` into).
///
/// `Ok(None)` — no artifact env, legacy boot, caller proceeds in `cwd`.
/// `Ok(Some(dir))` — extracted (or already-cached / last-good) app root.
/// `Err(_)` — fail closed: the artifact was requested but is misconfigured
/// (no/invalid hash) or could not be fetched and nothing is cached.
pub fn prepare() -> Result<Option<PathBuf>, String> {
    let Some(cfg) = cfg_from_env() else {
        return Ok(None);
    };
    // Fail closed: an artifact deploy MUST carry a verifiable hash. A missing
    // or malformed hash is a misconfiguration, never a silent
    // extract-without-verify.
    if !is_sha256_hex(&cfg.sha256) {
        return Err(format!(
            "PYLON_ARTIFACT_ID is set but PYLON_ARTIFACT_SHA256 is missing/invalid \
             (need 64 lowercase hex chars); refusing to boot an unverified bundle"
        ));
    }
    let root = ensure(&cfg)?;

    // Monorepo deploy: the bundle is the whole workspace (so `workspace:*` deps
    // resolve), and the app lives in a subdirectory. Chdir into it — node
    // resolution then walks UP to the workspace-root `node_modules` for hoisted
    // deps, and `app.ts` / `app/` are found relative to cwd. PYLON_APP_SUBDIR is
    // set by the trusted control plane (the app's path relative to the workspace
    // root); unset → single-app bundle, run at the extracted root.
    let Some(subdir) = non_empty(std::env::var("PYLON_APP_SUBDIR").ok()) else {
        return Ok(Some(root));
    };
    let subdir = subdir.trim_matches('/');
    if subdir.is_empty() || subdir.split('/').any(|c| c == "..") {
        return Err(format!(
            "PYLON_APP_SUBDIR={subdir:?} is invalid (empty or contains '..')"
        ));
    }
    let app_dir = root.join(subdir);
    if !app_dir.is_dir() {
        return Err(format!(
            "PYLON_APP_SUBDIR={subdir:?} does not exist in the deployed bundle at {}",
            root.display()
        ));
    }
    Ok(Some(app_dir))
}

fn ensure(cfg: &Cfg) -> Result<PathBuf, String> {
    let target = cfg.base.join(&cfg.id);

    // 1. Cache hit — this exact artifact is already extracted. The common
    //    case on every restart after the first boot.
    if target.join(COMPLETE_MARKER).is_file() {
        return Ok(target);
    }

    // 2. Cache miss — fetch + verify + extract. On any failure, fall back to
    //    the most recent previously-good artifact rather than crash a machine
    //    that was serving fine before a Tigris/network blip.
    match fetch_and_install(cfg) {
        Ok(dir) => {
            prune_old(&cfg.base, &cfg.id);
            Ok(dir)
        }
        Err(e) => match newest_complete(&cfg.base, Some(&cfg.id)) {
            Some(prev) => {
                eprintln!(
                    "[artifact] fetch/extract for {} failed ({e}); booting last-good {}",
                    cfg.id,
                    prev.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                );
                Ok(prev)
            }
            None => Err(format!(
                "artifact {} fetch/extract failed and no cached bundle to fall back to: {e}",
                cfg.id
            )),
        },
    }
}

/// Download the bundle to a temp file, verify its SHA-256, and extract it
/// atomically into `<base>/<id>/`.
fn fetch_and_install(cfg: &Cfg) -> Result<PathBuf, String> {
    fs::create_dir_all(&cfg.base)
        .map_err(|e| format!("create artifact base {}: {e}", cfg.base.display()))?;
    // Clear any `.download-*` / `.staging-*` debris a previous crashed boot may
    // have left on the size-bounded volume.
    sweep_debris(&cfg.base);

    // Prune old artifacts BEFORE downloading/unpacking the new one so a full
    // volume can self-heal: drop stale bundles first, then install into the
    // reclaimed room. Pruning only after a successful install would wedge the
    // machine permanently — once the volume fills, the unpack fails, prune never
    // runs, and every later boot re-fails identically ("failed to unpack … No
    // space left on device"). (The post-install prune still runs to drop the
    // bundle this deploy supersedes.)
    prune_old(&cfg.base, &cfg.id);

    let url = resolve_url(cfg)?;
    // Per-pid temp name so overlapping boots don't clobber each other.
    let pid = std::process::id();
    let tarball = cfg.base.join(format!(".download-{}-{pid}.tar.gz", cfg.id));
    let _tarball_guard = ScopedRemove(tarball.clone());
    download_to_file(&url, &tarball)?;

    // Integrity: the expected hash arrives via the trusted machine-config env,
    // so verifying the downloaded bytes defeats a tampered/corrupt object even
    // if object storage is compromised. (`prepare` already proved it's 64-hex.)
    let actual = sha256_file(&tarball)?;
    if actual != cfg.sha256 {
        return Err(format!(
            "sha256 mismatch: expected {}, got {actual}",
            cfg.sha256
        ));
    }

    install_bundle(cfg, &tarball)
    // `_tarball_guard` removes the temp here, on every path.
}

/// Resolve the download url: GET `token_url` (which mints a fresh signed url)
/// when set, else use the pre-signed `url` directly.
fn resolve_url(cfg: &Cfg) -> Result<String, String> {
    if !cfg.token_url.is_empty() {
        let resp = ureq::get(&cfg.token_url)
            .timeout(std::time::Duration::from_secs(30))
            .call()
            .map_err(|e| format!("mint artifact url ({}): {e}", cfg.token_url))?;
        // Bound the body so a hostile/buggy endpoint can't OOM boot.
        let mut body = String::new();
        resp.into_reader()
            .take(MAX_TOKEN_BODY_BYTES)
            .read_to_string(&mut body)
            .map_err(|e| format!("read artifact-url response: {e}"))?;
        let parsed: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("parse artifact-url response: {e}"))?;
        let url = parsed
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "artifact-url response had no `url`".to_string())?;
        return Ok(url.to_string());
    }
    if cfg.url.is_empty() {
        return Err("neither PYLON_ARTIFACT_URL nor PYLON_ARTIFACT_TOKEN_URL set".into());
    }
    Ok(cfg.url.clone())
}

fn download_to_file(url: &str, dest: &Path) -> Result<(), String> {
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(600))
        .call()
        .map_err(|e| format!("download artifact: {e}"))?;
    // `.take` caps the stream so an unbounded/hostile body can't fill /data.
    let mut reader = resp.into_reader().take(MAX_DOWNLOAD_BYTES + 1);
    let mut file = fs::File::create(dest).map_err(|e| format!("create {}: {e}", dest.display()))?;
    let written =
        std::io::copy(&mut reader, &mut file).map_err(|e| format!("stream artifact: {e}"))?;
    if written > MAX_DOWNLOAD_BYTES {
        return Err(format!(
            "bundle exceeds {} byte download cap",
            MAX_DOWNLOAD_BYTES
        ));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("read for hash: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in digest {
        let _ = write!(hex, "{b:02x}");
    }
    Ok(hex)
}

/// Extract the gzip-tar `tarball` into a per-pid staging dir (size-capped,
/// permission-sanitized, path-traversal-guarded by the `tar` crate), then
/// atomically rename it to `<base>/<id>/` and write the completion marker
/// LAST so a dir-without-marker (a torn extraction) is re-done on next boot.
fn install_bundle(cfg: &Cfg, tarball: &Path) -> Result<PathBuf, String> {
    let target = cfg.base.join(&cfg.id);
    let pid = std::process::id();
    let staging = cfg.base.join(format!(".staging-{}-{pid}", cfg.id));

    // Unpack into staging, retrying ONCE after an aggressive space reclaim if the
    // volume runs out of room mid-extract. On a full disk we'd rather drop the
    // local rollback cache (re-fetchable from object storage) than leave the
    // deploy wedged — so the new bundle always wins the last of the space.
    let mut freed_for_space = false;
    loop {
        if staging.exists() {
            fs::remove_dir_all(&staging)
                .map_err(|e| format!("clean staging {}: {e}", staging.display()))?;
        }
        fs::create_dir_all(&staging)
            .map_err(|e| format!("create staging {}: {e}", staging.display()))?;

        match unpack_archive(tarball, &staging) {
            Ok(()) => break,
            Err(e) if !freed_for_space && is_no_space(&e) => {
                eprintln!(
                    "[artifact] out of space unpacking {}; reclaiming other bundles and retrying",
                    cfg.id
                );
                let _ = fs::remove_dir_all(&staging);
                free_artifact_space(&cfg.base, &staging);
                freed_for_space = true;
                // loop: retry the unpack with the reclaimed room.
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(e);
            }
        }
    }
    let _staging_guard = ScopedRemoveDir(staging.clone());

    // A concurrent boot may have won the race and already produced the target.
    if target.join(COMPLETE_MARKER).is_file() {
        return Ok(target);
    }
    // Remove an incomplete prior target (no marker, else we'd have cache-hit).
    if target.exists() {
        fs::remove_dir_all(&target)
            .map_err(|e| format!("remove stale {}: {e}", target.display()))?;
    }
    fs::rename(&staging, &target).map_err(|e| {
        format!(
            "promote {} → {}: {e} (staging + target must be on the same filesystem)",
            staging.display(),
            target.display()
        )
    })?;
    // Renamed away — nothing for the guard to remove.
    std::mem::forget(_staging_guard);

    fs::write(target.join(COMPLETE_MARKER), cfg.sha256.as_bytes())
        .map_err(|e| format!("write completion marker: {e}"))?;
    Ok(target)
}

/// Remove stale `.download-*` / `.staging-*` debris left by crashed boots.
fn sweep_debris(base: &Path) {
    let Ok(rd) = fs::read_dir(base) else {
        return;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".download-") {
            let _ = fs::remove_file(entry.path());
        } else if name.starts_with(".staging-") {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// The newest (by mtime) fully-extracted artifact dir under `base`, optionally
/// excluding `exclude_id`. Used for last-good fallback + rollback. Cache-hit
/// semantics mean a dir is extracted (and its mtime set) exactly once, so
/// mtime tracks deploy order in practice.
fn newest_complete(base: &Path, exclude_id: Option<&str>) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(base).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') || Some(name) == exclude_id {
            continue;
        }
        if !path.join(COMPLETE_MARKER).is_file() {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
            best = Some((mtime, path));
        }
    }
    best.map(|(_, p)| p)
}

/// Keep `KEEP_ARTIFACTS` most-recent COMPLETE artifacts (always including
/// `keep_id`); remove older complete ones AND any torn (marker-less) dirs to
/// bound `/data` use and clear failed extractions.
fn prune_old(base: &Path, keep_id: &str) {
    let Ok(rd) = fs::read_dir(base) else {
        return;
    };
    let mut complete: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if name.starts_with('.') {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if path.join(COMPLETE_MARKER).is_file() {
            complete.push((mtime, path));
        } else if name != keep_id {
            // Torn extraction from a crashed boot — never a rollback target.
            let _ = fs::remove_dir_all(&path);
        }
    }
    complete.sort_by(|a, b| b.0.cmp(&a.0)); // newest first
    let mut kept = 0usize;
    for (_, path) in complete {
        let is_current = path.file_name().and_then(|n| n.to_str()) == Some(keep_id);
        if is_current || kept < KEEP_ARTIFACTS {
            if !is_current {
                kept += 1;
            }
            continue;
        }
        let _ = fs::remove_dir_all(&path);
    }
}

/// Unpack the gzip-tar `tarball` into `staging`, enforcing the decompressed-size
/// cap (decompression-bomb guard) and the tar crate's path-traversal/symlink
/// defenses. Split out of `install_bundle` so the caller can retry it after
/// reclaiming space. Does NOT preserve permissions/ownership: a bundle is JS +
/// assets and has no business carrying setuid/setgid/world-writable bits.
fn unpack_archive(tarball: &Path, staging: &Path) -> Result<(), String> {
    let file = fs::File::open(tarball).map_err(|e| format!("open bundle: {e}"))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive.set_preserve_permissions(false);
    archive.set_preserve_ownerships(false);
    archive.set_overwrite(true);

    let mut total: u64 = 0;
    let entries = archive
        .entries()
        .map_err(|e| format!("read bundle entries: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("read bundle entry: {e}"))?;
        total = total.saturating_add(entry.size());
        if total > MAX_EXTRACT_BYTES {
            return Err(format!(
                "bundle exceeds {} byte extracted-size cap",
                MAX_EXTRACT_BYTES
            ));
        }
        entry
            .unpack_in(staging)
            .map_err(|e| format!("unpack entry into {}: {e}", staging.display()))?;
    }
    Ok(())
}

/// Does this error string indicate the volume ran out of space?
fn is_no_space(err: &str) -> bool {
    err.contains("No space left") || err.contains("os error 28") || err.contains("ENOSPC")
}

/// Last-resort reclaim when an unpack runs the volume out of space: remove every
/// extracted artifact dir under `base` except the in-progress `staging` (and
/// other in-flight `.staging-*` / `.download-*` temps, which their own guards
/// own). The local rollback cache is sacrificed — it's re-fetchable from object
/// storage — so the deploy in flight can finish rather than wedge a full disk.
fn free_artifact_space(base: &Path, staging: &Path) {
    let Ok(rd) = fs::read_dir(base) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_dir() || path == *staging {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".staging-") || name.starts_with(".download-") {
            continue;
        }
        let _ = fs::remove_dir_all(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_bundle(files: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let tar_path = dir.path().join("bundle.tar.gz");
        let f = fs::File::create(&tar_path).unwrap();
        let enc = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        let mut builder = tar::Builder::new(enc);
        for (name, data) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        let sha = sha256_file(&tar_path).unwrap();
        (dir, tar_path, sha)
    }

    fn cfg_for(base: &Path, id: &str, sha: &str) -> Cfg {
        Cfg {
            id: id.to_string(),
            sha256: sha.to_string(),
            url: String::new(),
            token_url: String::new(),
            base: base.to_path_buf(),
        }
    }

    #[test]
    fn install_bundle_extracts_tree_and_writes_marker() {
        let base = tempfile::tempdir().unwrap();
        let (_b, tarball, sha) = make_bundle(&[
            ("app.ts", b"export default {}"),
            ("public/logo.png", b"\x89PNG"),
        ]);
        let cfg = cfg_for(base.path(), "dep-1", &sha);
        let dir = install_bundle(&cfg, &tarball).unwrap();
        assert_eq!(dir, base.path().join("dep-1"));
        assert_eq!(
            fs::read_to_string(dir.join("app.ts")).unwrap(),
            "export default {}"
        );
        assert!(dir.join("public/logo.png").is_file());
        assert!(dir.join(COMPLETE_MARKER).is_file());
        // No staging debris left behind.
        let debris = fs::read_dir(base.path())
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(".staging-"));
        assert!(!debris, "staging dir should be renamed away, not left");
    }

    #[test]
    fn is_sha256_hex_validation() {
        assert!(is_sha256_hex(&"a".repeat(64)));
        assert!(is_sha256_hex(&"0123456789abcdef".repeat(4)));
        assert!(!is_sha256_hex(""));
        assert!(!is_sha256_hex(&"a".repeat(63)));
        assert!(!is_sha256_hex(&"g".repeat(64))); // non-hex
                                                  // Uppercase hex is technically valid; cfg_from_env lowercases before
                                                  // this is ever called, so callers only see lowercase.
        assert!(is_sha256_hex(&"A".repeat(64)));
    }

    #[test]
    fn ensure_cache_hit_skips_install() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("dep-cached");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("app.ts"), b"x").unwrap();
        fs::write(dir.join(COMPLETE_MARKER), b"").unwrap();
        let cfg = cfg_for(base.path(), "dep-cached", &"a".repeat(64));
        assert_eq!(ensure(&cfg).unwrap(), dir);
    }

    #[test]
    fn ensure_falls_back_to_last_good_on_fetch_failure() {
        let base = tempfile::tempdir().unwrap();
        let good = base.path().join("dep-old");
        fs::create_dir_all(&good).unwrap();
        fs::write(good.join(COMPLETE_MARKER), b"").unwrap();
        let cfg = cfg_for(base.path(), "dep-new", &"a".repeat(64));
        assert_eq!(ensure(&cfg).unwrap(), good);
    }

    #[test]
    fn ensure_fails_closed_with_no_cache() {
        let base = tempfile::tempdir().unwrap();
        let cfg = cfg_for(base.path(), "dep-new", &"a".repeat(64));
        assert!(ensure(&cfg).is_err());
    }

    #[test]
    fn sha_mismatch_is_detected() {
        let (_b, tarball, real) = make_bundle(&[("app.ts", b"x")]);
        let actual = sha256_file(&tarball).unwrap();
        assert_eq!(actual, real);
        assert_ne!(actual, "b".repeat(64));
    }

    #[test]
    fn sweep_removes_debris_only() {
        let base = tempfile::tempdir().unwrap();
        fs::write(base.path().join(".download-x-1.tar.gz"), b"junk").unwrap();
        fs::create_dir_all(base.path().join(".staging-x-1")).unwrap();
        let keep = base.path().join("dep-1");
        fs::create_dir_all(&keep).unwrap();
        fs::write(keep.join(COMPLETE_MARKER), b"").unwrap();
        sweep_debris(base.path());
        assert!(!base.path().join(".download-x-1.tar.gz").exists());
        assert!(!base.path().join(".staging-x-1").exists());
        assert!(keep.exists(), "real artifact untouched");
    }

    #[test]
    fn prune_keeps_recent_current_and_removes_torn() {
        let base = tempfile::tempdir().unwrap();
        for i in 0..6 {
            let d = base.path().join(format!("dep-{i}"));
            fs::create_dir_all(&d).unwrap();
            let mut f = fs::File::create(d.join(COMPLETE_MARKER)).unwrap();
            f.write_all(b"").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // A torn (marker-less) dir that is NOT the current id → must be removed.
        fs::create_dir_all(base.path().join("dep-torn")).unwrap();
        prune_old(base.path(), "dep-0");
        assert!(base.path().join("dep-0").exists(), "current kept");
        assert!(!base.path().join("dep-torn").exists(), "torn removed");
        let complete = fs::read_dir(base.path())
            .unwrap()
            .flatten()
            .filter(|e| e.path().join(COMPLETE_MARKER).is_file())
            .count();
        assert_eq!(complete, KEEP_ARTIFACTS + 1);
    }

    #[test]
    fn is_no_space_matches_enospc() {
        assert!(is_no_space(
            "unpack entry into /data/.staging-x: No space left on device (os error 28)"
        ));
        assert!(is_no_space(
            "stream artifact: No space left on device (os error 28)"
        ));
        assert!(!is_no_space("sha256 mismatch: expected a, got b"));
        assert!(!is_no_space("read bundle entry: unexpected EOF"));
    }

    #[test]
    fn free_artifact_space_clears_others_keeps_staging_and_temps() {
        let base = tempfile::tempdir().unwrap();
        // Two old extracted artifacts (the reclaimable rollback cache).
        for id in ["dep-old1", "dep-old2"] {
            let d = base.path().join(id);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join(COMPLETE_MARKER), b"").unwrap();
        }
        // The in-progress staging for the deploy we're installing.
        let staging = base.path().join(".staging-dep-new-1");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("app.ts"), b"x").unwrap();
        // Another boot's download temp — left to its own guard.
        fs::write(base.path().join(".download-dep-new-2.tar.gz"), b"junk").unwrap();

        free_artifact_space(base.path(), &staging);

        assert!(
            !base.path().join("dep-old1").exists(),
            "old artifact reclaimed"
        );
        assert!(
            !base.path().join("dep-old2").exists(),
            "old artifact reclaimed"
        );
        assert!(
            staging.join("app.ts").is_file(),
            "in-progress staging preserved"
        );
        assert!(
            base.path().join(".download-dep-new-2.tar.gz").exists(),
            "other in-flight temp left alone"
        );
    }
}
