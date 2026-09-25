//! `pylon shards build` — compile the Cargo crates that app.ts names in
//! `shard({ crate })` to WebAssembly and write each module to the shard's
//! `wasm` path.
//!
//! `pylon dev` runs the same build at start and again when a crate's Rust
//! source changes. `pylon build` and `pylon deploy` ship the `wasm` files as
//! they are, so the Cloud builder needs no Rust toolchain.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use pylon_kernel::{AppManifest, ExitCode};

use crate::bun::run_bun_codegen;
use crate::output::print_diagnostics;

const WASM_TARGET: &str = "wasm32-unknown-unknown";

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let sub = args
        .iter()
        .skip_while(|a| *a != "shards")
        .nth(1)
        .map(String::as_str);
    match sub {
        Some("build") => {}
        _ => {
            eprintln!("Usage: pylon shards build [app.ts]");
            return ExitCode::Usage;
        }
    }
    let entry = args
        .iter()
        .skip_while(|a| *a != "build")
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| "app.ts".into());
    let manifest_json = match run_bun_codegen(&entry, false) {
        Ok(json) => json,
        Err(d) => {
            print_diagnostics(&[d], json_mode);
            return ExitCode::Error;
        }
    };
    let manifest: AppManifest = match serde_json::from_str(&manifest_json) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: the manifest from {entry} does not parse: {e}");
            return ExitCode::Error;
        }
    };
    let app_root = Path::new(&entry)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    if !manifest.shards.iter().any(|s| s.crate_dir.is_some()) {
        let msg = "no shard in app.ts sets `crate`; nothing to build";
        if json_mode {
            println!("{}", serde_json::json!({ "built": [], "message": msg }));
        } else {
            println!("{msg}");
        }
        return ExitCode::Ok;
    }
    match build_all(&manifest, &app_root, json_mode) {
        Ok(built) => {
            if json_mode {
                let built: Vec<String> = built.iter().map(|p| p.display().to_string()).collect();
                println!("{}", serde_json::json!({ "built": built }));
            }
            ExitCode::Ok
        }
        Err(e) => {
            if json_mode {
                println!("{}", serde_json::json!({ "error": e }));
            } else {
                eprintln!("error: {e}");
            }
            ExitCode::Error
        }
    }
}

/// Build every shard crate once and copy each module to the `wasm` paths
/// that use it. Returns the written paths.
pub fn build_all(
    manifest: &AppManifest,
    app_root: &Path,
    json_mode: bool,
) -> Result<Vec<PathBuf>, String> {
    // One crate can back several kinds (same module, other settings).
    let mut by_crate: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for def in &manifest.shards {
        if let Some(dir) = &def.crate_dir {
            by_crate
                .entry(dir.as_str())
                .or_default()
                .push(def.wasm.as_str());
        }
    }
    let mut written = Vec::new();
    for (crate_dir, outputs) in by_crate {
        let module = build_crate(&app_root.join(crate_dir), json_mode)?;
        for out in outputs {
            let dest = app_root.join(out);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
            // `wasm` may name cargo's own output; copying a file onto itself
            // truncates it on some platforms.
            let same = match (module.canonicalize(), dest.canonicalize()) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            };
            if !same {
                std::fs::copy(&module, &dest).map_err(|e| {
                    format!(
                        "cannot copy {} to {}: {e}",
                        module.display(),
                        dest.display()
                    )
                })?;
            }
            if !json_mode {
                println!("  ✓ shard module {}", dest.display());
            }
            written.push(dest);
        }
    }
    Ok(written)
}

/// `cargo build --release --target wasm32-unknown-unknown` in `dir`.
/// Returns the path of the `.wasm` cdylib cargo reports.
fn build_crate(dir: &Path, json_mode: bool) -> Result<PathBuf, String> {
    let manifest_path = dir.join("Cargo.toml");
    if !manifest_path.is_file() {
        return Err(format!("{} has no Cargo.toml", dir.display()));
    }
    if !json_mode {
        println!("  building shard crate {}", dir.display());
    }
    let mut child = Command::new("cargo")
        .args(["build", "--release", "--target", WASM_TARGET])
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--message-format=json-render-diagnostics")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "cargo is not installed; install Rust from https://rustup.rs, then run `rustup target add wasm32-unknown-unknown`".to_string()
            } else {
                format!("cannot run cargo: {e}")
            }
        })?;

    // Drain stderr on its own thread while stdout is read below, so neither
    // pipe fills and stalls cargo. Outside JSON mode, echo it as it comes.
    let stderr_pipe = child.stderr.take();
    let stderr_thread = std::thread::spawn(move || {
        let mut kept = String::new();
        if let Some(pipe) = stderr_pipe {
            for line in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
                if !json_mode {
                    eprintln!("{line}");
                }
                kept.push_str(&line);
                kept.push('\n');
            }
        }
        kept
    });

    // The build's own messages are JSON lines on stdout. Take the cdylib
    // artifact of the crate named by --manifest-path, wherever the target
    // dir is; a dependency can be a cdylib too.
    let want = manifest_path
        .canonicalize()
        .unwrap_or_else(|_| manifest_path.clone());
    let mut module = None;
    if let Some(stdout) = child.stdout.take() {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if msg["reason"] != "compiler-artifact" {
                continue;
            }
            let is_ours = msg["manifest_path"]
                .as_str()
                .map(|p| {
                    Path::new(p)
                        .canonicalize()
                        .unwrap_or_else(|_| PathBuf::from(p))
                })
                .is_some_and(|p| p == want);
            let is_cdylib = msg["target"]["kind"]
                .as_array()
                .is_some_and(|k| k.iter().any(|v| v == "cdylib"));
            if !is_ours || !is_cdylib {
                continue;
            }
            if let Some(files) = msg["filenames"].as_array() {
                for f in files.iter().filter_map(|f| f.as_str()) {
                    if f.ends_with(".wasm") {
                        module = Some(PathBuf::from(f));
                    }
                }
            }
        }
    }
    let status = child
        .wait()
        .map_err(|e| format!("cargo did not finish: {e}"))?;
    let stderr = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        if json_mode && !stderr.is_empty() {
            eprintln!("{stderr}");
        }
        let hint = if stderr.contains("target may not be installed")
            || stderr.contains("can't find crate for `core`")
            || stderr.contains("can't find crate for `std`")
        {
            "; run `rustup target add wasm32-unknown-unknown`"
        } else {
            ""
        };
        return Err(format!("cargo build failed in {}{hint}", dir.display()));
    }
    module.ok_or_else(|| {
        format!(
            "cargo built {} but produced no .wasm; add `[lib] crate-type = [\"cdylib\"]` to its Cargo.toml",
            dir.display()
        )
    })
}

/// Files `pylon dev` watches for shards: each module, and each crate's
/// sources (`Cargo.toml` and `.rs` files outside `target/`).
#[derive(Debug, Default, Clone)]
pub struct ShardWatch {
    pub modules: Vec<PathBuf>,
    pub crates: Vec<PathBuf>,
}

impl ShardWatch {
    pub fn new(manifest: &AppManifest, app_root: &Path) -> Self {
        // The watcher reports canonical paths. Canonicalize the root, not
        // each file: a module may not exist yet.
        let root = std::fs::canonicalize(app_root).unwrap_or_else(|_| app_root.to_path_buf());
        let join = |rel: &str| {
            let p = root.join(rel);
            std::fs::canonicalize(&p).unwrap_or(p)
        };
        Self {
            modules: manifest.shards.iter().map(|s| join(&s.wasm)).collect(),
            crates: manifest
                .shards
                .iter()
                .filter_map(|s| s.crate_dir.as_deref())
                .map(join)
                .collect(),
        }
    }

    /// A Rust source or Cargo.toml inside a shard crate, outside `target/`.
    pub fn is_crate_source(&self, path: &Path) -> bool {
        let is_source = path.extension().is_some_and(|e| e == "rs")
            || path.file_name().is_some_and(|n| n == "Cargo.toml");
        is_source
            && self.crates.iter().any(|dir| {
                path.strip_prefix(dir)
                    .is_ok_and(|rel| !rel.components().any(|c| c.as_os_str() == "target"))
            })
    }

    pub fn is_module(&self, path: &Path) -> bool {
        self.modules.iter().any(|m| m == path)
    }

    /// Modification times of every watched file, for the poll fallback.
    pub fn mtimes(&self) -> Vec<(PathBuf, Option<std::time::SystemTime>)> {
        let mut out = Vec::new();
        for m in &self.modules {
            out.push((m.clone(), mtime(m)));
        }
        for dir in &self.crates {
            collect_sources(self, dir, &mut out, 0);
        }
        out.sort();
        out
    }
}

fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn collect_sources(
    watch: &ShardWatch,
    dir: &Path,
    out: &mut Vec<(PathBuf, Option<std::time::SystemTime>)>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n == "target" || n == ".git")
            {
                continue;
            }
            collect_sources(watch, &path, out, depth + 1);
        } else if watch.is_crate_source(&path) {
            out.push((path.clone(), mtime(&path)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_sources_exclude_target() {
        let watch = ShardWatch {
            modules: vec![PathBuf::from("/app/shards/arena.wasm")],
            crates: vec![PathBuf::from("/app/shards/arena")],
        };
        assert!(watch.is_crate_source(Path::new("/app/shards/arena/src/lib.rs")));
        assert!(watch.is_crate_source(Path::new("/app/shards/arena/Cargo.toml")));
        assert!(
            !watch.is_crate_source(Path::new("/app/shards/arena/target/release/build/x/out.rs"))
        );
        assert!(!watch.is_crate_source(Path::new("/app/other/lib.rs")));
        assert!(!watch.is_crate_source(Path::new("/app/shards/arena/README.md")));
        assert!(watch.is_module(Path::new("/app/shards/arena.wasm")));
    }
}
