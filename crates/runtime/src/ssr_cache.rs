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
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
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

// ---- In-memory layer over the on-disk cache (hot-path latency) ----
//
// A cache HIT otherwise costs two `fs::read`s (`.html` + `.meta`) plus a JSON
// parse PER request. At the SSR cache's measured hit throughput (~23k rps on
// the Rust-only path) that syscall + parse tax is the dominant cost. This
// RwLock-fronted map sits in front of disk: a hit takes the READ lock and
// clones an `Arc<MemEntry>` (concurrent, no syscall) — only a miss-populate or
// an eviction takes the write lock. Entries are immutable per build + TTL'd, so
// there's deliberately NO recency-update on read (which would force a write and
// defeat the RwLock); eviction is oldest-inserted (FIFO) once over the byte
// budget. Freshness is recomputed per get from the stored `rendered_at`, so a
// stale entry is still surfaced (stale-on-error relies on that).
//
// Coherence: every write goes through `put()` (disk + mem together) and the
// build namespace is fixed for the process, so the key space is unambiguous and
// a new deploy is a new process with an empty map. Bounded by
// `PYLON_SSR_MEMCACHE_BYTES` (default 32 MiB; `0` disables → pure disk).
struct MemEntry {
    status: u16,
    headers: Vec<(String, String)>,
    body: Arc<Vec<u8>>,
    rendered_at: u64,
    revalidate_secs: u64,
    seq: u64,
}

struct MemInner {
    map: HashMap<String, Arc<MemEntry>>,
    bytes: usize,
    seq: u64,
}

struct MemCache {
    inner: RwLock<MemInner>,
    budget_bytes: usize,
}

impl MemCache {
    fn get(&self, key: &str) -> Option<Arc<MemEntry>> {
        self.inner.read().unwrap().map.get(key).cloned()
    }

    fn put(
        &self,
        key: &str,
        status: u16,
        headers: &[(String, String)],
        body: &[u8],
        rendered_at: u64,
        revalidate_secs: u64,
    ) {
        // Never let one oversized page evict the whole working set — it serves
        // from disk instead.
        if body.len() > self.budget_bytes {
            return;
        }
        let mut g = self.inner.write().unwrap();
        g.seq += 1;
        let entry = Arc::new(MemEntry {
            status,
            headers: headers.to_vec(),
            body: Arc::new(body.to_vec()),
            rendered_at,
            revalidate_secs,
            seq: g.seq,
        });
        if let Some(old) = g.map.insert(key.to_string(), Arc::clone(&entry)) {
            g.bytes = g.bytes.saturating_sub(old.body.len());
        }
        g.bytes += entry.body.len();
        // Evict oldest-inserted (FIFO) until under budget. The map is small
        // (budget / page-size) so the min-scan on the rare overflow is cheap.
        while g.bytes > self.budget_bytes {
            let victim = g
                .map
                .iter()
                .min_by_key(|(_, v)| v.seq)
                .map(|(k, _)| k.clone());
            match victim {
                Some(k) => {
                    if let Some(v) = g.map.remove(&k) {
                        g.bytes = g.bytes.saturating_sub(v.body.len());
                    }
                }
                None => break,
            }
        }
    }

    fn clear(&self) {
        let mut g = self.inner.write().unwrap();
        g.map.clear();
        g.bytes = 0;
    }
}

fn mem_cache() -> Option<&'static MemCache> {
    static MEM: std::sync::OnceLock<Option<MemCache>> = std::sync::OnceLock::new();
    MEM.get_or_init(|| {
        let budget = std::env::var("PYLON_SSR_MEMCACHE_BYTES")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(32 * 1024 * 1024);
        if budget == 0 {
            return None;
        }
        Some(MemCache {
            inner: RwLock::new(MemInner {
                map: HashMap::new(),
                bytes: 0,
                seq: 0,
            }),
            budget_bytes: budget,
        })
    })
    .as_ref()
}

/// Look up a cached render. Returns None on any miss / read error (the caller
/// then renders live — a corrupt or partial entry must never 500 the page).
/// Consults the in-memory layer first (no syscall); a disk hit is promoted into
/// it so the next request skips disk too (warm-after-restart).
pub fn get(route_path: &str, pathname: &str, vary: &[(String, String)]) -> Option<CacheEntry> {
    let key = cache_key(route_path, pathname, vary);
    if let Some(mem) = mem_cache() {
        if let Some(e) = mem.get(&key) {
            let age = now_secs().saturating_sub(e.rendered_at);
            return Some(CacheEntry {
                status: e.status,
                headers: e.headers.clone(),
                body: (*e.body).clone(),
                fresh: age < e.revalidate_secs,
            });
        }
    }
    let dir = cache_dir();
    let body = std::fs::read(dir.join(format!("{key}.html"))).ok()?;
    let meta_raw = std::fs::read(dir.join(format!("{key}.meta"))).ok()?;
    let meta: Meta = serde_json::from_slice(&meta_raw).ok()?;
    let rendered_at = meta.rendered_at;
    let revalidate_secs = meta.revalidate_secs;
    let status = meta.status;
    let headers = meta.headers;
    if let Some(mem) = mem_cache() {
        mem.put(&key, status, &headers, &body, rendered_at, revalidate_secs);
    }
    let age = now_secs().saturating_sub(rendered_at);
    Some(CacheEntry {
        status,
        headers,
        body,
        fresh: age < revalidate_secs,
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
    let rendered_at = now_secs();
    // Populate the in-memory layer too — the just-rendered page is hot, so the
    // first cookie-anonymous reader after this serves from RAM, not disk.
    if let Some(mem) = mem_cache() {
        mem.put(&key, status, headers, body, rendered_at, revalidate_secs);
    }
    let dir = cache_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let meta = Meta {
        status,
        revalidate_secs,
        rendered_at,
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
    // Keep the on-disk namespace bounded (throttled readdir-based eviction) so
    // path enumeration can't fill the disk.
    maybe_sweep_disk(&dir);
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

/// On-disk namespace bounds: `(max_entries, max_bytes)`. Without these, a
/// cacheable dynamic/catch-all route lets a client enumerate distinct paths and
/// write one `.html`+`.meta` pair each, forever — filling the disk and wedging
/// the machine (`wipe_stale_namespaces` only reclaims OTHER deploys). The
/// in-memory layer is already bounded; this mirrors that to disk. A bound of
/// `0` disables it. Read once.
fn disk_cache_limits() -> (usize, u64) {
    static LIMITS: std::sync::OnceLock<(usize, u64)> = std::sync::OnceLock::new();
    *LIMITS.get_or_init(|| {
        let entries = std::env::var("PYLON_SSR_DISK_CACHE_MAX_ENTRIES")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(4096);
        let bytes = std::env::var("PYLON_SSR_DISK_CACHE_MAX_BYTES")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(512 * 1024 * 1024);
        (entries, bytes)
    })
}

/// Run the disk sweep on roughly 1 write in `DISK_SWEEP_INTERVAL`. The sweep
/// does a readdir + stat, so amortize it rather than paying per write; between
/// sweeps the namespace can overshoot the cap by at most this many entries —
/// a bounded, negligible excess relative to the cap.
const DISK_SWEEP_INTERVAL: u64 = 64;

fn maybe_sweep_disk(dir: &Path) {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // n == 0 (first write) sweeps too, so the bound holds from the start.
    if n % DISK_SWEEP_INTERVAL != 0 {
        return;
    }
    let (max_entries, max_bytes) = disk_cache_limits();
    if max_entries == 0 && max_bytes == 0 {
        return; // both bounds disabled
    }
    sweep_disk_cache(dir, max_entries, max_bytes);
}

/// Evict oldest-by-mtime `.html`/`.meta` pairs until the namespace is within
/// BOTH `max_entries` and `max_bytes` (`0` = that bound disabled). Best-effort:
/// the cache is a perf layer, so any error just leaves the entry in place. A
/// concurrently-read victim degrades to a live re-render (get() returns None on
/// a missing `.meta`), never a 500.
fn sweep_disk_cache(dir: &Path, max_entries: usize, max_bytes: u64) {
    struct Entry {
        html: PathBuf,
        meta: PathBuf,
        mtime: SystemTime,
        bytes: u64,
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    let mut entries: Vec<Entry> = Vec::new();
    let mut total_bytes: u64 = 0;
    for ent in rd.flatten() {
        let path = ent.path();
        // One record per completed `.html`; `.meta` is counted with its pair,
        // and in-flight `.tmp.*` files (and anything else) are ignored.
        if path.extension().and_then(|e| e.to_str()) != Some("html") {
            continue;
        }
        let md = match ent.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let meta_path = path.with_extension("meta");
        let meta_bytes = std::fs::metadata(&meta_path).map(|m| m.len()).unwrap_or(0);
        let bytes = md.len() + meta_bytes;
        total_bytes += bytes;
        entries.push(Entry {
            html: path,
            meta: meta_path,
            mtime: md.modified().unwrap_or(UNIX_EPOCH),
            bytes,
        });
    }
    let over = (max_entries != 0 && entries.len() > max_entries)
        || (max_bytes != 0 && total_bytes > max_bytes);
    if !over {
        return;
    }
    entries.sort_by_key(|e| e.mtime); // oldest first
    let mut count = entries.len();
    for e in &entries {
        let within = (max_entries == 0 || count <= max_entries)
            && (max_bytes == 0 || total_bytes <= max_bytes);
        if within {
            break;
        }
        let _ = std::fs::remove_file(&e.html);
        let _ = std::fs::remove_file(&e.meta);
        count -= 1;
        total_bytes = total_bytes.saturating_sub(e.bytes);
    }
}

/// At boot, remove cache namespaces from previous deploys so old HTML can't be
/// served and the disk doesn't grow unbounded across deploys. Keeps only the
/// current build's dir. No-op when there's no cache yet.
pub fn wipe_stale_namespaces() {
    // Drop any in-memory entries too (defensive — at boot the map is empty, but
    // this keeps mem and disk in lockstep if ever called mid-process).
    if let Some(mem) = mem_cache() {
        mem.clear();
    }
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

    fn fresh_mem(budget: usize) -> MemCache {
        MemCache {
            inner: RwLock::new(MemInner {
                map: HashMap::new(),
                bytes: 0,
                seq: 0,
            }),
            budget_bytes: budget,
        }
    }

    #[test]
    fn mem_layer_serves_recomputes_freshness_and_evicts_oldest() {
        let m = fresh_mem(1024);
        let h = vec![("content-type".to_string(), "text/html".to_string())];

        // Put a FRESH entry (large TTL) — get returns it, fresh, byte-identical.
        m.put("a", 200, &h, b"AAAA", now_secs(), 600);
        let e = m.get("a").expect("hit");
        assert_eq!(e.status, 200);
        assert_eq!(&*e.body, b"AAAA");

        // Freshness is recomputed PER get from rendered_at, so a TTL-0 entry
        // reads back STALE (stale-on-error depends on this) — but is still
        // present, not evicted.
        m.put("b", 200, &h, b"BB", now_secs(), 0);
        let b = m.get("b").expect("stale entry still cached");
        assert!(now_secs().saturating_sub(b.rendered_at) >= b.revalidate_secs);

        // A body larger than the whole budget is NOT cached (it would evict the
        // working set) — it serves from disk instead.
        m.put("huge", 200, &h, &vec![0u8; 2048], now_secs(), 600);
        assert!(m.get("huge").is_none());

        // Byte-budget eviction is oldest-inserted (FIFO): fill past 1024 with
        // 300-byte bodies; the earliest keys are evicted first.
        let m2 = fresh_mem(1024);
        for i in 0..5 {
            m2.put(&format!("k{i}"), 200, &h, &vec![b'x'; 300], now_secs(), 600);
        }
        // 5 * 300 = 1500 > 1024 → at most 3 survive (3*300=900 ≤ 1024).
        let survivors = (0..5)
            .filter(|i| m2.get(&format!("k{i}")).is_some())
            .count();
        assert!(survivors <= 3, "budget enforced (survivors={survivors})");
        assert!(m2.get("k4").is_some(), "newest survives");
        assert!(m2.get("k0").is_none(), "oldest evicted first");
    }

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

    /// Unique temp dir for the sweep tests — they operate on a dir directly
    /// (no env / process-global state), so they need no ENV_LOCK.
    fn unique_tmp(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pylon_ssr_sweep_{tag}_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn disk_sweep_evicts_oldest_until_within_entry_cap() {
        use std::time::Duration;
        let dir = unique_tmp("entries");
        // 10 entries with strictly increasing mtimes so eviction order is
        // deterministic (oldest = key0 ... newest = key9).
        let t0 = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for i in 0..10u64 {
            let html = dir.join(format!("key{i}.html"));
            std::fs::write(&html, vec![b'x'; 100]).unwrap();
            std::fs::write(dir.join(format!("key{i}.meta")), b"{}").unwrap();
            let f = std::fs::File::options().write(true).open(&html).unwrap();
            f.set_modified(t0 + Duration::from_secs(i)).unwrap();
        }

        // Cap at 4 entries; byte bound disabled.
        sweep_disk_cache(&dir, 4, 0);

        let remaining: Vec<u64> = (0..10)
            .filter(|i| dir.join(format!("key{i}.html")).exists())
            .collect();
        assert_eq!(
            remaining,
            vec![6, 7, 8, 9],
            "4 newest survive, oldest evicted"
        );
        assert!(
            !dir.join("key0.meta").exists(),
            "evicted entry's paired .meta is removed too"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disk_sweep_enforces_byte_cap_and_is_noop_under_cap() {
        let dir = unique_tmp("bytes");
        // 5 entries of ~1000 bytes each (html 1000 + meta 2).
        for i in 0..5u64 {
            std::fs::write(dir.join(format!("k{i}.html")), vec![b'x'; 1000]).unwrap();
            std::fs::write(dir.join(format!("k{i}.meta")), b"{}").unwrap();
        }
        // Byte cap ~2.5 entries; entry bound disabled. Must drop to <= cap.
        sweep_disk_cache(&dir, 0, 2500);
        let survivors = (0..5)
            .filter(|i| dir.join(format!("k{i}.html")).exists())
            .count();
        assert!(survivors <= 3, "byte cap enforced (survivors={survivors})");

        // Under cap → no-op.
        let dir2 = unique_tmp("under");
        for i in 0..3u64 {
            std::fs::write(dir2.join(format!("k{i}.html")), b"x").unwrap();
            std::fs::write(dir2.join(format!("k{i}.meta")), b"{}").unwrap();
        }
        sweep_disk_cache(&dir2, 4096, 512 * 1024 * 1024);
        let count = (0..3)
            .filter(|i| dir2.join(format!("k{i}.html")).exists())
            .count();
        assert_eq!(count, 3, "nothing evicted when under both caps");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }
}
