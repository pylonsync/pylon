//! `pylon db copy` — copy a SQLite app's data into an empty Postgres
//! database, and the same copy at boot when `PYLON_IMPORT_FROM_SQLITE` is set.
//!
//! ```sh
//! pylon db copy --from /data/pylon.db --to postgres://… [--check]
//! ```
//!
//! `--to` defaults to `DATABASE_URL`. `--manifest` defaults to
//! `pylon.manifest.json` (or `PYLON_MANIFEST`). The auth, job and workflow
//! files are found the way the server finds them (see
//! `pylon_runtime::db_copy::SqliteSource::from_env`).

use pylon_kernel::ExitCode;
use pylon_runtime::db_copy::{self, CopyError, CopyMode, CopyOutcome, SqliteSource};

use crate::commands::args::flag_value;
use crate::output::{print_error, print_json};

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let Some(from) = flag_value(args, "--from") else {
        print_error("--from <path to the SQLite app database> is required");
        eprintln!("Usage: pylon db copy --from <pylon.db> [--to <postgres url>] [--check]");
        return ExitCode::Usage;
    };
    let Some(to) = flag_value(args, "--to").or_else(|| std::env::var("DATABASE_URL").ok()) else {
        print_error("--to <postgres url> is required when DATABASE_URL is not set");
        return ExitCode::Usage;
    };
    if !(to.starts_with("postgres://") || to.starts_with("postgresql://")) {
        print_error("--to must be a postgres:// or postgresql:// URL");
        return ExitCode::Usage;
    }
    let manifest_path = flag_value(args, "--manifest")
        .or_else(|| std::env::var("PYLON_MANIFEST").ok())
        .unwrap_or_else(|| "pylon.manifest.json".into());
    let mode = if args.iter().any(|a| a == "--check") {
        CopyMode::Check
    } else {
        CopyMode::Write
    };

    let manifest = match crate::manifest::load_manifest(&manifest_path) {
        Ok(m) => m,
        Err(diags) => {
            crate::output::print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    super::start::apply_pg_schema(&to, &manifest, json_mode);
    let runtime = match pylon_runtime::Runtime::open(&to, manifest) {
        Ok(rt) => rt,
        Err(e) => {
            print_error(&format!("could not open the Postgres database: {e}"));
            return ExitCode::Error;
        }
    };
    let source = SqliteSource::from_env(&from);
    let result = db_copy::copy(&runtime, &source, mode);
    pylon_runtime::pg_boot_guard::release();
    report(result, json_mode)
}

/// Run the copy during `pylon start`. Returns false when the server must
/// not start.
pub(crate) fn run_at_boot(
    runtime: &pylon_runtime::Runtime,
    sqlite_path: &str,
    json_mode: bool,
) -> bool {
    // A finished copy is the normal case on every boot after the first.
    match db_copy::is_copied(runtime) {
        Ok(true) => return true,
        Ok(false) => {}
        Err(e) => {
            print_error(&format!(
                "[start] could not check for an earlier SQLite copy: {e}"
            ));
            return false;
        }
    }
    if !json_mode {
        println!("  Copying SQLite data from {sqlite_path} into Postgres");
    }
    let source = SqliteSource::from_env(sqlite_path);
    let result = db_copy::copy(runtime, &source, CopyMode::Write);
    if let Err(e) = &result {
        if let Err(record_err) = db_copy::record_failure(runtime, e) {
            print_error(&format!(
                "[start] could not record the failed copy: {record_err}"
            ));
        }
    }
    matches!(report(result, json_mode), ExitCode::Ok)
}

fn report(result: Result<CopyOutcome, CopyError>, json_mode: bool) -> ExitCode {
    match result {
        Ok(outcome) => {
            if json_mode {
                print_json(&serde_json::json!({ "code": "DB_COPY_OK", "result": outcome }));
                return ExitCode::Ok;
            }
            match outcome {
                CopyOutcome::Copied(report) => {
                    println!("  Copied SQLite data into Postgres:");
                    for t in report.tables.iter().filter(|t| t.rows > 0) {
                        println!("    {:<32} {:>8} rows", t.table, t.rows);
                    }
                    println!("    {:<32} {:>8}", "jobs", report.jobs);
                    println!("    {:<32} {:>8}", "workflow runs", report.workflows);
                    println!("    {:<32} {:>8}", "change sequence", report.change_seq);
                    print_skipped(&report.skipped, &report.skipped_columns);
                }
                CopyOutcome::Checked {
                    tables,
                    skipped,
                    skipped_columns,
                } => {
                    println!("  All values convert. A copy now would write:");
                    for t in tables.iter().filter(|t| t.rows > 0) {
                        println!("    {:<32} {:>8} rows", t.table, t.rows);
                    }
                    print_skipped(&skipped, &skipped_columns);
                }
                CopyOutcome::AlreadyCopied { copied_at, source } => {
                    println!("  Already copied from {source} at {copied_at}. Nothing written.");
                }
            }
            ExitCode::Ok
        }
        Err(e) => {
            if json_mode {
                let (code, detail) = match &e {
                    CopyError::Problems { total, listed } => (
                        "DB_COPY_PROBLEMS",
                        serde_json::json!({ "total": total, "problems": listed }),
                    ),
                    CopyError::NotEmpty { tables } => (
                        "DB_COPY_TARGET_NOT_EMPTY",
                        serde_json::json!({ "tables": tables }),
                    ),
                    CopyError::Failed(m) => ("DB_COPY_FAILED", serde_json::json!({ "message": m })),
                };
                print_json(&serde_json::json!({ "code": code, "error": detail }));
            } else {
                print_error(&format!("SQLite copy failed: {e}"));
            }
            ExitCode::Error
        }
    }
}

fn print_skipped(skipped: &[db_copy::TableCount], columns: &[db_copy::SkippedColumn]) {
    if !skipped.is_empty() {
        println!("  Tables not copied (no entity in the manifest; the rows stay in SQLite):");
        for t in skipped {
            println!("    {:<32} {:>8} rows", t.table, t.rows);
        }
    }
    if !columns.is_empty() {
        println!("  Columns not copied (no field in the manifest; the values stay in SQLite):");
        for c in columns {
            let name = format!("{}.{}", c.table, c.column);
            println!(
                "    {:<32} {:>8} rows with a value",
                name, c.rows_with_value
            );
        }
    }
}
