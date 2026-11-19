//! Disk, database and file storage numbers for Studio's Health page. Each one
//! is read when an operator asks for it, never on a request path.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Free and total space of the file system that holds `path`. `None` off Unix
/// or when the system does not answer.
pub(crate) fn disk_json(path: &Path) -> Option<serde_json::Value> {
    let (total, free) = disk_space(path)?;
    Some(serde_json::json!({
        "path": path.display().to_string(),
        "total_bytes": total,
        "available_bytes": free,
    }))
}

#[cfg(unix)]
fn disk_space(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` is NUL-terminated and `st` is a zeroed statvfs that the call
    // fills.
    let st = unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return None;
        }
        st
    };
    let frag = st.f_frsize as u64;
    Some((st.f_blocks as u64 * frag, st.f_bavail as u64 * frag))
}

#[cfg(not(unix))]
fn disk_space(_path: &Path) -> Option<(u64, u64)> {
    None
}

/// The size of a SQLite database: the main file, the write-ahead log and the
/// shared-memory file.
pub(crate) fn sqlite_json(db_path: &str) -> serde_json::Value {
    let size = |suffix: &str| {
        std::fs::metadata(format!("{db_path}{suffix}"))
            .map(|m| m.len())
            .unwrap_or(0)
    };
    let file = size("");
    let wal = size("-wal");
    let shm = size("-shm");
    serde_json::json!({
        "path": db_path,
        "file_bytes": file,
        "wal_bytes": wal,
        "total_bytes": file + wal + shm,
    })
}

/// Pool use and database size of a Postgres database.
pub(crate) fn postgres_json(pool: &pylon_storage::pg_datastore::PgPool) -> serde_json::Value {
    let size = pool.database_size_bytes(Duration::from_millis(500));
    let max = pool.max_size();
    let idle = pool.available_count();
    let in_use = pool.in_use_count();
    serde_json::json!({
        "pool_size": max,
        "pool_open": idle + in_use,
        "pool_idle": idle,
        "pool_in_use": in_use,
        "database_bytes": size.as_ref().ok(),
        "error": size.err(),
    })
}

/// The count and size of the files in local file storage, from the last scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirTotals {
    pub files: u64,
    pub bytes: u64,
}

/// Count the files under `dir` and add up their sizes. The sidecar
/// directories that hold upload metadata (names that start with a dot) are
/// left out.
pub(crate) fn scan_dir(dir: &Path) -> DirTotals {
    let mut totals = DirTotals { files: 0, bytes: 0 };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                totals.files += 1;
                totals.bytes += meta.len();
            }
        }
    }
    totals
}

/// A scan that is older than this starts a new one.
const SCAN_MAX_AGE: Duration = Duration::from_secs(300);

struct ScanCache {
    dir: PathBuf,
    result: Option<(Instant, DirTotals)>,
}

static SCAN: Mutex<Option<ScanCache>> = Mutex::new(None);
static SCANNING: AtomicBool = AtomicBool::new(false);

/// The file storage report. Local storage reports the totals of the last scan
/// of its directory. A scan runs on its own thread when there is none or it
/// is more than five minutes old, so a directory of many files never holds
/// the request. Another provider reports only its name.
pub(crate) fn file_storage_json() -> serde_json::Value {
    let provider = std::env::var("PYLON_FILES_PROVIDER").unwrap_or_else(|_| "local".into());
    if provider != "local" {
        return serde_json::json!({ "provider": provider });
    }
    let dir = PathBuf::from(std::env::var("PYLON_FILES_DIR").unwrap_or_else(|_| "uploads".into()));
    let cached = {
        let guard = SCAN.lock().unwrap();
        guard
            .as_ref()
            .filter(|c| c.dir == dir)
            .and_then(|c| c.result)
    };
    let stale = cached.is_none_or(|(at, _)| at.elapsed() > SCAN_MAX_AGE);
    if stale && !SCANNING.swap(true, Ordering::AcqRel) {
        let scan_dir_path = dir.clone();
        let spawned = std::thread::Builder::new()
            .name("pylon-files-scan".into())
            .spawn(move || {
                let totals = scan_dir(&scan_dir_path);
                *SCAN.lock().unwrap() = Some(ScanCache {
                    dir: scan_dir_path,
                    result: Some((Instant::now(), totals)),
                });
                SCANNING.store(false, Ordering::Release);
            });
        if spawned.is_err() {
            SCANNING.store(false, Ordering::Release);
        }
    }
    match cached {
        Some((at, totals)) => serde_json::json!({
            "provider": "local",
            "dir": dir.display().to_string(),
            "files": totals.files,
            "bytes": totals.bytes,
            "scanned_secs_ago": at.elapsed().as_secs(),
        }),
        None => serde_json::json!({
            "provider": "local",
            "dir": dir.display().to_string(),
            "scanning": true,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scan_counts_files_and_skips_the_metadata_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), [0u8; 10]).unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("b.png"), [0u8; 5]).unwrap();
        std::fs::create_dir(dir.path().join(".ownership")).unwrap();
        std::fs::write(dir.path().join(".ownership").join("a.png.json"), b"{}").unwrap();
        assert_eq!(
            scan_dir(dir.path()),
            DirTotals {
                files: 2,
                bytes: 15
            }
        );
    }

    #[test]
    fn a_missing_directory_has_no_files() {
        let totals = scan_dir(Path::new("/nonexistent/pylon-scan-test"));
        assert_eq!(totals, DirTotals { files: 0, bytes: 0 });
    }

    #[cfg(unix)]
    #[test]
    fn the_disk_of_a_directory_reports_space() {
        let dir = tempfile::tempdir().unwrap();
        let disk = disk_json(dir.path()).expect("statvfs answers");
        assert!(disk["total_bytes"].as_u64().unwrap() > 0);
        assert!(disk["available_bytes"].as_u64().unwrap() <= disk["total_bytes"].as_u64().unwrap());
    }

    #[test]
    fn a_sqlite_size_adds_the_log_files() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("app.db");
        std::fs::write(&db, [0u8; 100]).unwrap();
        std::fs::write(dir.path().join("app.db-wal"), [0u8; 20]).unwrap();
        let v = sqlite_json(db.to_str().unwrap());
        assert_eq!(v["file_bytes"], 100);
        assert_eq!(v["wal_bytes"], 20);
        assert_eq!(v["total_bytes"], 120);
    }
}
