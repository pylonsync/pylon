//! Memory figures for `/metrics`.
//!
//! On Pylon Cloud each app runs alone in a Fly VM, so `/proc/meminfo` is the
//! machine's memory. When `MemAvailable` reaches zero the kernel OOM killer
//! picks a process: a Bun runner (the app loses its functions) or the pylon
//! process itself (Fly restarts the machine). Neither was visible before this
//! module; the first sign of trouble was the outage.
//!
//! Linux only. Every other platform returns `None` and `/metrics` omits the
//! block.

use std::path::Path;

/// One reading of machine and process memory, in bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySnapshot {
    /// `MemTotal` from /proc/meminfo.
    pub total_bytes: u64,
    /// `MemAvailable` from /proc/meminfo: memory the kernel can hand out
    /// without swapping.
    pub available_bytes: u64,
    /// Resident set size of the pylon process.
    pub process_rss_bytes: u64,
    /// Summed resident set size of the pylon process's direct children.
    /// These are the Bun function runners, plus any other subprocess the
    /// runtime has open at the time of the reading.
    pub children_rss_bytes: u64,
    /// Number of direct children counted in `children_rss_bytes`.
    pub children: u64,
}

impl MemorySnapshot {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "total_bytes": self.total_bytes,
            "available_bytes": self.available_bytes,
            "process_rss_bytes": self.process_rss_bytes,
            "children_rss_bytes": self.children_rss_bytes,
            "children": self.children,
        })
    }

    pub fn prometheus(&self) -> String {
        format!(
            "# HELP pylon_memory_total_bytes Machine memory (MemTotal).\n\
             # TYPE pylon_memory_total_bytes gauge\n\
             pylon_memory_total_bytes {}\n\
             # HELP pylon_memory_available_bytes Machine memory available (MemAvailable).\n\
             # TYPE pylon_memory_available_bytes gauge\n\
             pylon_memory_available_bytes {}\n\
             # HELP pylon_process_rss_bytes Resident memory of the pylon process.\n\
             # TYPE pylon_process_rss_bytes gauge\n\
             pylon_process_rss_bytes {}\n\
             # HELP pylon_children_rss_bytes Resident memory of the pylon process's child processes (function runners).\n\
             # TYPE pylon_children_rss_bytes gauge\n\
             pylon_children_rss_bytes {}\n\
             # HELP pylon_children Child processes counted in pylon_children_rss_bytes.\n\
             # TYPE pylon_children gauge\n\
             pylon_children {}\n",
            self.total_bytes,
            self.available_bytes,
            self.process_rss_bytes,
            self.children_rss_bytes,
            self.children,
        )
    }
}

/// Read the current memory figures. `None` off Linux, or when /proc/meminfo
/// cannot be read.
pub fn snapshot() -> Option<MemorySnapshot> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    snapshot_from(Path::new("/proc"), std::process::id())
}

/// `snapshot()` against an arbitrary procfs root, so the walk is testable.
fn snapshot_from(proc_root: &Path, self_pid: u32) -> Option<MemorySnapshot> {
    let meminfo = std::fs::read_to_string(proc_root.join("meminfo")).ok()?;
    let (total_bytes, available_bytes) = parse_meminfo(&meminfo)?;

    let process_rss_bytes =
        std::fs::read_to_string(proc_root.join(self_pid.to_string()).join("status"))
            .ok()
            .and_then(|s| parse_status(&s))
            .map(|p| p.rss_bytes)
            .unwrap_or(0);

    let mut children_rss_bytes = 0u64;
    let mut children = 0u64;
    if let Ok(entries) = std::fs::read_dir(proc_root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            // A process can exit between read_dir and this read; skip it.
            let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
                continue;
            };
            if let Some(p) = parse_status(&status) {
                if p.ppid == self_pid {
                    children_rss_bytes += p.rss_bytes;
                    children += 1;
                }
            }
        }
    }

    Some(MemorySnapshot {
        total_bytes,
        available_bytes,
        process_rss_bytes,
        children_rss_bytes,
        children,
    })
}

/// `(MemTotal, MemAvailable)` in bytes. Both lines are required.
fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let mut total = None;
    let mut available = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total = parse_kb(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available = parse_kb(rest);
        }
    }
    Some((total?, available?))
}

struct ProcStatus {
    ppid: u32,
    rss_bytes: u64,
}

/// `PPid` and `VmRSS` from /proc/<pid>/status. Kernel threads have no
/// `VmRSS` line; they count as 0 bytes.
fn parse_status(text: &str) -> Option<ProcStatus> {
    let mut ppid = None;
    let mut rss_bytes = 0;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("PPid:") {
            ppid = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("VmRSS:") {
            rss_bytes = parse_kb(rest).unwrap_or(0);
        }
    }
    Some(ProcStatus {
        ppid: ppid?,
        rss_bytes,
    })
}

/// "   123456 kB" → 123456 * 1024.
fn parse_kb(rest: &str) -> Option<u64> {
    let n: u64 = rest.trim().trim_end_matches("kB").trim().parse().ok()?;
    Some(n.saturating_mul(1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_reads_total_and_available() {
        let text = "MemTotal:        1004000 kB\nMemFree:          20000 kB\nMemAvailable:     150000 kB\n";
        assert_eq!(parse_meminfo(text), Some((1004000 * 1024, 150000 * 1024)));
    }

    #[test]
    fn meminfo_without_available_is_none() {
        assert_eq!(parse_meminfo("MemTotal: 1004000 kB\n"), None);
    }

    #[test]
    fn status_without_rss_counts_zero() {
        let p = parse_status("Name:\tkthreadd\nPPid:\t0\n").unwrap();
        assert_eq!(p.ppid, 0);
        assert_eq!(p.rss_bytes, 0);
    }

    #[test]
    fn snapshot_sums_direct_children_only() {
        let dir = std::env::temp_dir().join(format!("pylon-mem-test-{}", std::process::id()));
        let write = |pid: &str, ppid: u32, rss_kb: u64| {
            let d = dir.join(pid);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("status"),
                format!("Name:\tx\nPPid:\t{ppid}\nVmRSS:\t{rss_kb} kB\n"),
            )
            .unwrap();
        };
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meminfo"),
            "MemTotal: 1000 kB\nMemAvailable: 400 kB\n",
        )
        .unwrap();
        write("10", 1, 300); // pylon itself
        write("11", 10, 100); // runner
        write("12", 10, 50); // runner
        write("13", 11, 999); // grandchild: not counted
        std::fs::create_dir_all(dir.join("self")).unwrap(); // non-numeric: skipped

        let snap = snapshot_from(&dir, 10).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(
            snap,
            MemorySnapshot {
                total_bytes: 1000 * 1024,
                available_bytes: 400 * 1024,
                process_rss_bytes: 300 * 1024,
                children_rss_bytes: 150 * 1024,
                children: 2,
            }
        );
    }
}
