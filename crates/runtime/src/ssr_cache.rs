//! On-disk ISR cache for proven-anonymous SSR pages (#277 Stage 2).
//!
//! A render that proved itself anonymous-safe + opted into caching (#277
//! Stage 1, the `x-pylon-cacheable` proof) is teed to
//! `.pylon/.cache/ssr/<build>/<key>.html` + `<key>.meta`. A cookie-anonymous
//! request then serves it straight off disk — no Bun round trip — when fresh;
//! a stale or missing entry re-renders live (which rewrites the entry), and the
//! `stale-while-revalidate` Cache-Control from Stage 1 lets CloudFlare absorb
//! the tail at the edge.
//!
//! Keyed by `<build>/SHA-256(route_path, pathname, allowlisted search params)`.
//! The build dir is `PYLON_ARTIFACT_ID` (cloud) or `"dev"` — a new deploy is a
//! new namespace, so a deploy can NEVER serve a prior deploy's HTML, and stale
//! namespaces are wiped at boot.
//!
//! SAFETY: only the host writes here, and only when the render emitted the
//! Stage-1 anonymity proof AND the request carried no resolving session cookie
//! AND no Set-Cookie was emitted — so a personalized/authed response can never
//! be stored or replayed.

use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Per-deploy namespace. The artifact id on cloud; `"dev"` otherwise (a dev
/// restart keeps the same namespace, which is fine — entries TTL out).
fn build_namespace() -> String {
    std::env::var("PYLON_ARTIFACT_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "dev".to_string())
}

/// Root holding all build namespaces: `.pylon/.cache/ssr` under cwd (cwd is the
/// artifact root on cloud). `PYLON_SSR_CACHE_ROOT` overrides it — useful to
/// pin the cache to a specific volume path, and used by tests to avoid touching
/// the process cwd.
fn cache_base() -> PathBuf {
    if let Some(root) = std::env::var_os("PYLON_SSR_CACHE_ROOT").filter(|s| !s.is_empty()) {
        return PathBuf::from(root);
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".pylon")
        .join(".cache")
        .join("ssr")
}

/// `<base>/<build>` — the current deploy's namespace dir.
fn cache_dir() -> PathBuf {
    cache_base().join(build_namespace())
}

/// SHA-256 hex of (route_path, pathname, sorted allowlisted params). The build
/// namespace lives in the dir, not the key.
pub fn cache_key(route_path: &str, pathname: &str, vary: &[(String, String)]) -> String {
    let mut h = Sha256::new();
    h.update(route_path.as_bytes());
    h.update([0]);
    h.update(pathname.as_bytes());
    let mut sorted: Vec<&(String, String)> = vary.iter().collect();
    sorted.sort();
    for (k, v) in sorted {
        h.update([0]);
        h.update(k.as_bytes());
        h.update(b"=");
        h.update(v.as_bytes());
    }
    format!("{:x}", h.finalize())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Meta {
    status: u16,
    revalidate_secs: u64,
    rendered_at: u64, // unix seconds
    headers: Vec<(String, String)>,
}

/// A cache entry read off disk.
pub struct CacheEntry {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// True when within the revalidate window. A stale entry re-renders live.
    pub fresh: bool,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Look up a cached render. Returns None on any miss / read error (the caller
/// then renders live — a corrupt or partial entry must never 500 the page).
pub fn get(route_path: &str, pathname: &str, vary: &[(String, String)]) -> Option<CacheEntry> {
    let key = cache_key(route_path, pathname, vary);
    let dir = cache_dir();
    let body = std::fs::read(dir.join(format!("{key}.html"))).ok()?;
    let meta_raw = std::fs::read(dir.join(format!("{key}.meta"))).ok()?;
    let meta: Meta = serde_json::from_slice(&meta_raw).ok()?;
    let age = now_secs().saturating_sub(meta.rendered_at);
    Some(CacheEntry {
        status: meta.status,
        headers: meta.headers,
        body,
        fresh: age < meta.revalidate_secs,
    })
}

/// Write a render to the cache (atomic: tmp + rename, so a reader never sees a
/// half-written file). `headers` are the FINAL response headers minus the
/// internal proof. Best-effort — a write failure is logged + ignored (the page
/// already streamed to the client).
pub fn put(
    route_path: &str,
    pathname: &str,
    vary: &[(String, String)],
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
    revalidate_secs: u64,
) {
    let key = cache_key(route_path, pathname, vary);
    let dir = cache_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let meta = Meta {
        status,
        revalidate_secs,
        rendered_at: now_secs(),
        headers: headers.to_vec(),
    };
    let meta_json = match serde_json::to_vec(&meta) {
        Ok(j) => j,
        Err(_) => return,
    };
    // Write body then meta, each atomically; meta last so a reader that sees
    // the meta is guaranteed the body is fully present.
    if atomic_write(&dir.join(format!("{key}.html")), body).is_err() {
        return;
    }
    let _ = atomic_write(&dir.join(format!("{key}.meta")), &meta_json);
}

fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!(
        "tmp.{}",
        // A per-write suffix so concurrent writers don't clobber each other's
        // tmp file. now-nanos is monotonic enough here (collisions just retry).
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// At boot, remove cache namespaces from previous deploys so old HTML can't be
/// served and the disk doesn't grow unbounded across deploys. Keeps only the
/// current build's dir. No-op when there's no cache yet.
pub fn wipe_stale_namespaces() {
    let root = cache_base();
    let current = build_namespace();
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy() != current {
            // Not the current deploy's namespace — drop it.
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Single-flight guard for the write-tee: dedupe concurrent writes of the same
/// key so two simultaneous renders don't both write (the first wins; the rest
/// skip). Returns true if THIS caller acquired the write slot.
fn write_guard() -> &'static Mutex<std::collections::HashSet<String>> {
    static G: std::sync::OnceLock<Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    G.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// Try to claim the write slot for a key. Returns a guard that releases on drop
/// (or None if another writer holds it).
pub fn try_claim_write(
    route_path: &str,
    pathname: &str,
    vary: &[(String, String)],
) -> Option<WriteClaim> {
    let key = cache_key(route_path, pathname, vary);
    let mut set = write_guard().lock().unwrap();
    if set.insert(key.clone()) {
        Some(WriteClaim { key })
    } else {
        None
    }
}

/// Releases the single-flight write slot on drop.
pub struct WriteClaim {
    key: String,
}
impl Drop for WriteClaim {
    fn drop(&mut self) {
        write_guard().lock().unwrap().remove(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The fs tests mutate process-global env (PYLON_SSR_CACHE_ROOT /
    // PYLON_ARTIFACT_ID); serialize them so cargo's parallel runner can't have
    // one clobber the other's namespace mid-test.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn key_is_deterministic_and_param_order_independent() {
        let a = cache_key(
            "/docs/:slug",
            "/docs/intro",
            &[("tab".into(), "x".into()), ("page".into(), "2".into())],
        );
        // Same params in a different order → same key (sorted).
        let b = cache_key(
            "/docs/:slug",
            "/docs/intro",
            &[("page".into(), "2".into()), ("tab".into(), "x".into())],
        );
        assert_eq!(a, b);
        // Different pathname → different key.
        let c = cache_key("/docs/:slug", "/docs/other", &[]);
        assert_ne!(a, c);
        // 64 hex chars (sha-256).
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|x| x.is_ascii_hexdigit()));
    }

    #[test]
    fn put_then_get_roundtrips_and_ttl_marks_stale() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Point the cache at a temp dir via the env override so the test never
        // mutates the process cwd (which would race parallel tests). The lock +
        // a fixed-name dir (removed at the end) keep this hermetic.
        let dir = std::env::temp_dir().join("pylon-ssr-cache-rt");
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("PYLON_SSR_CACHE_ROOT", &dir);
        std::env::remove_var("PYLON_ARTIFACT_ID");

        let headers = vec![("content-type".to_string(), "text/html".to_string())];
        put("/p", "/p", &[], 200, &headers, b"<html>hi</html>", 60);
        let e = get("/p", "/p", &[]).expect("hit");
        assert_eq!(e.status, 200);
        assert_eq!(e.body, b"<html>hi</html>");
        assert!(e.fresh);
        assert_eq!(e.headers, headers);

        // A zero-TTL entry is immediately stale.
        put("/q", "/q", &[], 200, &headers, b"x", 0);
        assert!(!get("/q", "/q", &[]).unwrap().fresh);

        // Miss for an unknown path.
        assert!(get("/nope", "/nope", &[]).is_none());

        std::env::remove_var("PYLON_SSR_CACHE_ROOT");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wipe_drops_prior_build_namespaces_only() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join("pylon-ssr-cache-wipe");
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("PYLON_SSR_CACHE_ROOT", &dir);
        std::env::set_var("PYLON_ARTIFACT_ID", "build-A");

        let headers = vec![("content-type".to_string(), "text/html".to_string())];
        put("/p", "/p", &[], 200, &headers, b"A", 60);
        assert!(get("/p", "/p", &[]).is_some());

        // A "new deploy" — bump the namespace, then wipe. The old build's dir
        // is removed; the current build's lookups simply miss (cold), and a
        // stray namespace from a third build is also dropped.
        std::fs::create_dir_all(dir.join("build-OLD")).unwrap();
        std::env::set_var("PYLON_ARTIFACT_ID", "build-B");
        wipe_stale_namespaces();
        assert!(!dir.join("build-A").exists(), "prior build dir wiped");
        assert!(!dir.join("build-OLD").exists(), "stray build dir wiped");
        // build-B is the current namespace; it's preserved (even if empty).
        put("/p", "/p", &[], 200, &headers, b"B", 60);
        assert_eq!(get("/p", "/p", &[]).unwrap().body, b"B");
        assert!(dir.join("build-B").exists(), "current build dir kept");

        std::env::remove_var("PYLON_SSR_CACHE_ROOT");
        std::env::remove_var("PYLON_ARTIFACT_ID");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
