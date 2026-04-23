//! `pylon backup` and `pylon restore` — ship the database + uploads + manifest
//! in and out of a single directory bundle.
//!
//! A backup bundle is just a directory:
//!
//! ```text
//! <bundle>/
//!   VERSION        — pylon version used to produce this bundle
//!   pylon.db     — copy of the SQLite database
//!   pylon.db-wal — WAL file (if any) so the backup is consistent
//!   manifest.json  — copy of the app manifest
//!   uploads/       — everything under PYLON_FILES_DIR
//! ```
//!
//! Usage:
//!
//! ```sh
//! pylon backup ./backups/2026-04-19
//! pylon restore ./backups/2026-04-19
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use pylon_kernel::ExitCode;

use crate::output::{print_error, print_json};

const VERSION_FILE: &str = "VERSION";
const DB_FILE: &str = "pylon.db";
const MANIFEST_FILE: &str = "manifest.json";
const UPLOADS_DIR: &str = "uploads";

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

pub fn run_backup(args: &[String], json_mode: bool) -> ExitCode {
    let target = match args.iter().skip(1).find(|a| !a.starts_with('-')) {
        Some(p) => p.clone(),
        None => {
            print_error("backup requires a target directory");
            eprintln!("  Usage: pylon backup <dir>");
            return ExitCode::Usage;
        }
    };

    let db_path = resolve_db_path();
    let manifest_path = resolve_manifest_path();
    let uploads_dir = resolve_uploads_dir();

    if let Err(e) = create_dir_all(&target) {
        print_error(&format!("Cannot create {target}: {e}"));
        return ExitCode::Error;
    }

    // Write VERSION.
    if let Err(e) = fs::write(
        Path::new(&target).join(VERSION_FILE),
        pylon_kernel::VERSION,
    ) {
        print_error(&format!("Cannot write VERSION: {e}"));
        return ExitCode::Error;
    }

    // Copy DB (and WAL/SHM if present).
    let mut copied_db = false;
    if Path::new(&db_path).exists() {
        for ext in ["", "-wal", "-shm"] {
            let src = format!("{db_path}{ext}");
            if !Path::new(&src).exists() {
                continue;
            }
            let dst = Path::new(&target).join(format!("{DB_FILE}{ext}"));
            if let Err(e) = fs::copy(&src, &dst) {
                print_error(&format!("Cannot copy {src}: {e}"));
                return ExitCode::Error;
            }
            if ext.is_empty() {
                copied_db = true;
            }
        }
    }

    // Copy manifest.
    let mut copied_manifest = false;
    if Path::new(&manifest_path).exists() {
        let dst = Path::new(&target).join(MANIFEST_FILE);
        if let Err(e) = fs::copy(&manifest_path, &dst) {
            print_error(&format!("Cannot copy manifest: {e}"));
            return ExitCode::Error;
        }
        copied_manifest = true;
    }

    // Copy uploads tree.
    let mut copied_files = 0u64;
    if Path::new(&uploads_dir).exists() {
        let uploads_target = Path::new(&target).join(UPLOADS_DIR);
        if let Err(e) = copy_tree(&PathBuf::from(&uploads_dir), &uploads_target, &mut copied_files) {
            print_error(&format!("Cannot copy uploads: {e}"));
            return ExitCode::Error;
        }
    }

    if json_mode {
        print_json(&serde_json::json!({
            "ok": true,
            "target": target,
            "db": copied_db,
            "manifest": copied_manifest,
            "uploaded_files": copied_files,
        }));
    } else {
        eprintln!("Backup written to {target}");
        eprintln!("  db:        {copied_db}");
        eprintln!("  manifest:  {copied_manifest}");
        eprintln!("  files:     {copied_files}");
    }

    ExitCode::Ok
}

pub fn run_restore(args: &[String], json_mode: bool) -> ExitCode {
    let source = match args.iter().skip(1).find(|a| !a.starts_with('-')) {
        Some(p) => p.clone(),
        None => {
            print_error("restore requires a source directory");
            eprintln!("  Usage: pylon restore <dir>");
            return ExitCode::Usage;
        }
    };

    let confirm_flag =
        args.iter().any(|a| a == "--yes" || a == "-y");

    if !Path::new(&source).exists() {
        print_error(&format!("Source directory not found: {source}"));
        return ExitCode::Error;
    }

    let db_path = resolve_db_path();
    let manifest_path = resolve_manifest_path();
    let uploads_dir = resolve_uploads_dir();

    if !confirm_flag {
        eprintln!("Restoring from {source} will OVERWRITE:");
        eprintln!("  {db_path}");
        eprintln!("  {manifest_path}");
        eprintln!("  {uploads_dir}");
        eprintln!();
        eprint!("Continue? [y/N] ");
        use std::io::{self, BufRead, Write};
        let _ = io::stderr().flush();
        let mut line = String::new();
        let _ = io::stdin().lock().read_line(&mut line);
        let answer = line.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            eprintln!("Aborted.");
            return ExitCode::Ok;
        }
    }

    // Restore DB.
    let mut restored_db = false;
    for ext in ["", "-wal", "-shm"] {
        let src = Path::new(&source).join(format!("{DB_FILE}{ext}"));
        if !src.exists() {
            // Remove any matching extension on the target so we don't leave
            // a stale WAL that confuses SQLite.
            let dst = format!("{db_path}{ext}");
            let _ = fs::remove_file(&dst);
            continue;
        }
        let dst = format!("{db_path}{ext}");
        if let Err(e) = fs::copy(&src, &dst) {
            print_error(&format!("Cannot restore db: {e}"));
            return ExitCode::Error;
        }
        if ext.is_empty() {
            restored_db = true;
        }
    }

    // Restore manifest.
    let mut restored_manifest = false;
    let manifest_src = Path::new(&source).join(MANIFEST_FILE);
    if manifest_src.exists() {
        if let Err(e) = fs::copy(&manifest_src, &manifest_path) {
            print_error(&format!("Cannot restore manifest: {e}"));
            return ExitCode::Error;
        }
        restored_manifest = true;
    }

    // Restore uploads.
    let mut restored_files = 0u64;
    let uploads_src = Path::new(&source).join(UPLOADS_DIR);
    if uploads_src.exists() {
        let _ = fs::remove_dir_all(&uploads_dir); // clean replace
        if let Err(e) = copy_tree(&uploads_src, &PathBuf::from(&uploads_dir), &mut restored_files) {
            print_error(&format!("Cannot restore uploads: {e}"));
            return ExitCode::Error;
        }
    }

    if json_mode {
        print_json(&serde_json::json!({
            "ok": true,
            "source": source,
            "db": restored_db,
            "manifest": restored_manifest,
            "restored_files": restored_files,
        }));
    } else {
        eprintln!("Restore complete.");
        eprintln!("  db:        {restored_db}");
        eprintln!("  manifest:  {restored_manifest}");
        eprintln!("  files:     {restored_files}");
    }

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_db_path() -> String {
    std::env::var("PYLON_DB_PATH").unwrap_or_else(|_| "pylon.db".to_string())
}

fn resolve_manifest_path() -> String {
    std::env::var("PYLON_MANIFEST").unwrap_or_else(|_| "pylon.manifest.json".to_string())
}

fn resolve_uploads_dir() -> String {
    std::env::var("PYLON_FILES_DIR").unwrap_or_else(|_| "uploads".to_string())
}

fn create_dir_all(path: &str) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

fn copy_tree(src: &Path, dst: &Path, counter: &mut u64) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_tree(&src_path, &dst_path, counter)?;
        } else if ty.is_file() {
            fs::copy(&src_path, &dst_path)?;
            *counter += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_tree_roundtrip() {
        let tmp = std::env::temp_dir()
            .join(format!("pylon_backup_test_{}", std::process::id()));
        let src = tmp.join("src");
        let dst = tmp.join("dst");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "hello").unwrap();
        fs::write(src.join("sub/b.txt"), "world").unwrap();

        let mut counter = 0u64;
        copy_tree(&src, &dst, &mut counter).unwrap();
        assert_eq!(counter, 2);
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
        assert_eq!(fs::read_to_string(dst.join("sub/b.txt")).unwrap(), "world");

        let _ = fs::remove_dir_all(&tmp);
    }
}
