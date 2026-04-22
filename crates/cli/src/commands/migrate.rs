use std::path::Path;

use statecraft_core::ExitCode;

use crate::output;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MIGRATIONS_DIR: &str = "migrations";

const MIGRATE_SUBCOMMANDS: [&str; 6] = ["create", "list", "status", "plan", "apply", "auto"];

const MIGRATIONS_TABLE: &str = "_statecraft_migrations";

// ---------------------------------------------------------------------------
// Entry point — dispatches to subcommands
// ---------------------------------------------------------------------------

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    let confirm_flag = args.iter().any(|a| a == "--yes" || a == "-y");
    let allow_destructive = args.iter().any(|a| a == "--allow-destructive");
    let hints = parse_rename_hints(args);

    match positional.get(1).copied() {
        Some("create") => run_create(&positional, json_mode),
        Some("list") => run_list(json_mode),
        Some("status") => run_status(json_mode),
        Some("plan") => run_plan(&hints, json_mode),
        Some("apply") => run_apply(&hints, confirm_flag, allow_destructive, json_mode),
        Some("auto") => run_apply(&hints, true, allow_destructive, json_mode),
        Some(sub) => {
            output::print_error(&format!("unknown migrate subcommand: \"{sub}\""));
            if let Some(suggestion) = suggest_subcommand(sub) {
                eprintln!("  Did you mean: {suggestion}?");
            }
            eprintln!();
            print_migrate_usage();
            ExitCode::Usage
        }
        None => {
            output::print_error("migrate requires a subcommand");
            eprintln!();
            print_migrate_usage();
            ExitCode::Usage
        }
    }
}

// ---------------------------------------------------------------------------
// migrate create <name>
// ---------------------------------------------------------------------------

fn run_create(positional: &[&str], json_mode: bool) -> ExitCode {
    let name = match positional.get(2) {
        Some(n) => *n,
        None => {
            output::print_error("migrate create requires a migration name");
            eprintln!("  Usage: statecraft migrate create <name>");
            return ExitCode::Usage;
        }
    };

    // Validate: name should be alphanumeric + underscores only.
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        output::print_error("migration name must contain only alphanumeric characters and underscores");
        return ExitCode::Usage;
    }

    let migrations_dir = Path::new(MIGRATIONS_DIR);
    if let Err(e) = std::fs::create_dir_all(migrations_dir) {
        output::print_error(&format!("failed to create migrations directory: {e}"));
        return ExitCode::Error;
    }

    let next_num = count_migrations(migrations_dir) + 1;
    let filename = format!("{:04}_{}.sql", next_num, name);
    let content = "-- UP\n\n-- DOWN\n";
    let filepath = migrations_dir.join(&filename);

    if let Err(e) = std::fs::write(&filepath, content) {
        output::print_error(&format!("failed to write migration file: {e}"));
        return ExitCode::Error;
    }

    if json_mode {
        output::print_json(&serde_json::json!({
            "created": filename,
            "path": filepath.display().to_string(),
            "number": next_num,
        }));
    } else {
        eprintln!("Created migration: {}", filepath.display());
    }

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// migrate list
// ---------------------------------------------------------------------------

fn run_list(json_mode: bool) -> ExitCode {
    let migrations_dir = Path::new(MIGRATIONS_DIR);

    let migrations = match read_migrations(migrations_dir) {
        Ok(m) => m,
        Err(msg) => {
            if json_mode {
                output::print_json(&serde_json::json!({ "migrations": [] }));
            } else {
                eprintln!("{msg}");
            }
            return ExitCode::Ok;
        }
    };

    if json_mode {
        let items: Vec<serde_json::Value> = migrations
            .iter()
            .map(|m| {
                serde_json::json!({
                    "number": m.number,
                    "name": m.name,
                    "filename": m.filename,
                    "status": "pending",
                })
            })
            .collect();
        output::print_json(&serde_json::json!({ "migrations": items }));
    } else {
        if migrations.is_empty() {
            eprintln!("No migrations found in {MIGRATIONS_DIR}/");
        } else {
            println!("{:<6} {:<12} {}", "NUM", "STATUS", "NAME");
            println!("{}", "-".repeat(40));
            for m in &migrations {
                // Without a DB connection we mark everything as pending.
                // A future enhancement can check _statecraft_migrations table.
                println!("{:04}   {:<12} {}", m.number, "pending", m.name);
            }
        }
    }

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// migrate status
// ---------------------------------------------------------------------------

fn run_status(json_mode: bool) -> ExitCode {
    let migrations_dir = Path::new(MIGRATIONS_DIR);

    let migrations = match read_migrations(migrations_dir) {
        Ok(m) => m,
        Err(msg) => {
            if json_mode {
                output::print_json(&serde_json::json!({
                    "total": 0,
                    "applied": 0,
                    "pending": 0,
                }));
            } else {
                eprintln!("{msg}");
            }
            return ExitCode::Ok;
        }
    };

    let total = migrations.len();
    // Without a live DB connection, all migrations are pending.
    let applied = 0usize;
    let pending = total;

    if json_mode {
        output::print_json(&serde_json::json!({
            "total": total,
            "applied": applied,
            "pending": pending,
        }));
    } else {
        println!("Migration status:");
        println!("  Total:   {total}");
        println!("  Applied: {applied}");
        println!("  Pending: {pending}");
    }

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct MigrationInfo {
    number: u32,
    name: String,
    filename: String,
}

/// Count existing `.sql` migration files in the given directory.
fn count_migrations(dir: &Path) -> u32 {
    read_migration_files(dir)
        .map(|files| files.len() as u32)
        .unwrap_or(0)
}

/// Read and sort migration `.sql` filenames from the directory.
fn read_migration_files(dir: &Path) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("Cannot read {}: {e}", dir.display()))?;

    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".sql") {
                Some(name)
            } else {
                None
            }
        })
        .collect();

    names.sort();
    Ok(names)
}

/// Parse migration filenames into structured info.
fn read_migrations(dir: &Path) -> Result<Vec<MigrationInfo>, String> {
    if !dir.exists() {
        return Err(format!("No migrations directory found ({MIGRATIONS_DIR}/)."));
    }

    let files = read_migration_files(dir)?;
    let mut migrations = Vec::new();

    for filename in files {
        if let Some(info) = parse_migration_filename(&filename) {
            migrations.push(info);
        }
    }

    Ok(migrations)
}

/// Parse a filename like `0001_add_users.sql` into a `MigrationInfo`.
fn parse_migration_filename(filename: &str) -> Option<MigrationInfo> {
    let stem = filename.strip_suffix(".sql")?;
    let underscore_pos = stem.find('_')?;
    let number: u32 = stem[..underscore_pos].parse().ok()?;
    let name = stem[underscore_pos + 1..].to_string();

    Some(MigrationInfo {
        number,
        name,
        filename: filename.to_string(),
    })
}

fn print_migrate_usage() {
    eprintln!("Usage: statecraft migrate <subcommand>");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  create <name>           Create a new SQL migration file");
    eprintln!("  list                    List all migrations and their status");
    eprintln!("  status                  Show current migration state");
    eprintln!("  plan                    Show diff between manifest and DB schema");
    eprintln!("  apply [--yes]           Apply pending schema changes");
    eprintln!("  auto                    Apply without prompting (alias for apply --yes)");
    eprintln!();
    eprintln!("Flags:");
    eprintln!("  --allow-destructive     Allow DROP TABLE and DROP COLUMN");
    eprintln!("  --yes, -y               Skip confirmation prompts");
    eprintln!("  --rename-table OLD:NEW  Treat a missing OLD entity as a rename to NEW");
    eprintln!("  --rename-column TBL.OLD:NEW  Treat a missing column as a rename");
}

// ---------------------------------------------------------------------------
// --rename-table and --rename-column flag parsing
// ---------------------------------------------------------------------------

fn parse_rename_hints(args: &[String]) -> statecraft_migrate::RenameHints {
    let mut hints = statecraft_migrate::RenameHints::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--rename-table" => {
                if let Some(spec) = args.get(i + 1) {
                    if let Some((from, to)) = spec.split_once(':') {
                        hints = hints.rename_table(from, to);
                    }
                    i += 2;
                    continue;
                }
            }
            "--rename-column" => {
                if let Some(spec) = args.get(i + 1) {
                    if let Some((tbl_field, to)) = spec.split_once(':') {
                        if let Some((tbl, from)) = tbl_field.split_once('.') {
                            hints = hints.rename_column(tbl, from, to);
                        }
                    }
                    i += 2;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    hints
}

// ---------------------------------------------------------------------------
// migrate plan — diff current manifest against DB
// ---------------------------------------------------------------------------

fn run_plan(hints: &statecraft_migrate::RenameHints, json_mode: bool) -> ExitCode {
    let (old, new) = match load_manifests() {
        Ok(x) => x,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };

    let plan = statecraft_migrate::diff_with_renames(&old, &new, hints);

    if json_mode {
        output::print_json(&serde_json::to_value(&plan).unwrap_or(serde_json::json!({})));
    } else if plan.is_empty() {
        eprintln!("No schema changes.");
    } else {
        eprintln!("Migration plan ({} steps):", plan.steps.len());
        for step in &plan.steps {
            let marker = if step.is_destructive() { " [DESTRUCTIVE]" } else { "" };
            eprintln!("  {}{marker}", step.sql());
        }
        if plan.has_destructive {
            eprintln!();
            eprintln!("Note: plan includes destructive operations. Use --allow-destructive with apply.");
        }
    }

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// migrate apply — execute the plan
// ---------------------------------------------------------------------------

fn run_apply(
    hints: &statecraft_migrate::RenameHints,
    skip_confirm: bool,
    allow_destructive: bool,
    json_mode: bool,
) -> ExitCode {
    let (old, new) = match load_manifests() {
        Ok(x) => x,
        Err(e) => {
            output::print_error(&e);
            return ExitCode::Error;
        }
    };

    let plan = statecraft_migrate::diff_with_renames(&old, &new, hints);

    if plan.is_empty() {
        if json_mode {
            output::print_json(&serde_json::json!({"applied": 0, "up_to_date": true}));
        } else {
            eprintln!("Already up to date.");
        }
        return ExitCode::Ok;
    }

    if plan.has_destructive && !allow_destructive {
        output::print_error("Plan contains destructive operations. Re-run with --allow-destructive to proceed.");
        for step in &plan.steps {
            if step.is_destructive() {
                eprintln!("  [DESTRUCTIVE] {}", step.sql());
            }
        }
        return ExitCode::Error;
    }

    if !skip_confirm {
        eprintln!("About to apply {} migration step(s):", plan.steps.len());
        for step in &plan.steps {
            eprintln!("  {}", step.sql());
        }
        eprintln!();
        eprint!("Continue? [y/N] ");
        use std::io::{self, BufRead, Write};
        let _ = io::stderr().flush();
        let stdin = io::stdin();
        let mut line = String::new();
        let _ = stdin.lock().read_line(&mut line);
        let answer = line.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            eprintln!("Aborted.");
            return ExitCode::Ok;
        }
    }

    let db_path = std::env::var("STATECRAFT_DB_PATH").unwrap_or_else(|_| "statecraft.db".into());
    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&format!("Failed to open database {db_path}: {e}"));
            return ExitCode::Error;
        }
    };

    if let Err(e) = ensure_migrations_table(&conn) {
        output::print_error(&e);
        return ExitCode::Error;
    }

    let mut applied: Vec<String> = Vec::new();
    for step in &plan.steps {
        if let Err(e) = conn.execute(step.sql(), []) {
            output::print_error(&format!("Step failed: {}\nSQL: {}", e, step.sql()));
            return ExitCode::Error;
        }
        applied.push(step.sql().to_string());
    }

    // Record the new manifest snapshot.
    let manifest_json = serde_json::to_string(&new).unwrap_or_default();
    let _ = conn.execute(
        &format!(
            "INSERT INTO {MIGRATIONS_TABLE} (applied_at, manifest) VALUES (?1, ?2)"
        ),
        rusqlite::params![
            chrono_now_iso(),
            manifest_json,
        ],
    );

    if json_mode {
        output::print_json(&serde_json::json!({
            "applied": applied.len(),
            "steps": applied,
        }));
    } else {
        eprintln!("Applied {} migration step(s).", applied.len());
    }

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// Manifest loading and DB state
// ---------------------------------------------------------------------------

fn load_manifests() -> Result<(statecraft_core::AppManifest, statecraft_core::AppManifest), String> {
    // Current (on-disk) manifest.
    let manifest_path = std::env::var("STATECRAFT_MANIFEST")
        .unwrap_or_else(|_| "statecraft.manifest.json".into());
    let current = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Cannot read manifest {manifest_path}: {e}"))?;
    let new: statecraft_core::AppManifest = serde_json::from_str(&current)
        .map_err(|e| format!("Invalid manifest JSON: {e}"))?;

    // Previously-applied manifest (from DB). Empty if fresh DB.
    let db_path = std::env::var("STATECRAFT_DB_PATH").unwrap_or_else(|_| "statecraft.db".into());
    let old = if std::path::Path::new(&db_path).exists() {
        match rusqlite::Connection::open(&db_path) {
            Ok(conn) => load_applied_manifest(&conn).unwrap_or_else(|_| empty_manifest()),
            Err(_) => empty_manifest(),
        }
    } else {
        empty_manifest()
    };

    Ok((old, new))
}

fn load_applied_manifest(conn: &rusqlite::Connection) -> Result<statecraft_core::AppManifest, String> {
    ensure_migrations_table(conn)?;

    let query = format!(
        "SELECT manifest FROM {MIGRATIONS_TABLE} ORDER BY applied_at DESC LIMIT 1"
    );
    let manifest_json: Option<String> = conn
        .query_row(&query, [], |row| row.get(0))
        .ok();

    match manifest_json {
        Some(json) => serde_json::from_str(&json)
            .map_err(|e| format!("Corrupted migration state: {e}")),
        None => Ok(empty_manifest()),
    }
}

fn ensure_migrations_table(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {MIGRATIONS_TABLE} (
            applied_at TEXT NOT NULL,
            manifest TEXT NOT NULL
        )"
    ))
    .map_err(|e| format!("Failed to create migrations table: {e}"))
}

fn empty_manifest() -> statecraft_core::AppManifest {
    statecraft_core::AppManifest {
        manifest_version: statecraft_core::MANIFEST_VERSION,
        name: "".into(),
        version: "".into(),
        entities: vec![],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
    }
}

fn chrono_now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{secs}")
}

/// Simple Levenshtein-based suggestion for migrate subcommands.
fn suggest_subcommand(input: &str) -> Option<&'static str> {
    let mut best: Option<(&str, usize)> = None;
    for &cmd in &MIGRATE_SUBCOMMANDS {
        let dist = levenshtein(input, cmd);
        if dist <= 3 {
            if best.map_or(true, |(_, d)| dist < d) {
                best = Some((cmd, dist));
            }
        }
    }
    best.map(|(cmd, _)| cmd)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_migration_filename() {
        let info = parse_migration_filename("0001_add_users.sql").unwrap();
        assert_eq!(info.number, 1);
        assert_eq!(info.name, "add_users");
        assert_eq!(info.filename, "0001_add_users.sql");
    }

    #[test]
    fn parse_migration_filename_high_number() {
        let info = parse_migration_filename("0042_create_orders.sql").unwrap();
        assert_eq!(info.number, 42);
        assert_eq!(info.name, "create_orders");
    }

    #[test]
    fn parse_migration_filename_no_extension() {
        assert!(parse_migration_filename("0001_test").is_none());
    }

    #[test]
    fn parse_migration_filename_no_underscore() {
        assert!(parse_migration_filename("0001.sql").is_none());
    }

    #[test]
    fn parse_migration_filename_bad_number() {
        assert!(parse_migration_filename("abcd_test.sql").is_none());
    }

    #[test]
    fn count_migrations_empty_dir() {
        let dir = std::env::temp_dir().join("statecraft_test_empty_migrations");
        let _ = std::fs::create_dir_all(&dir);
        // Clean any leftover files
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        assert_eq!(count_migrations(&dir), 0);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn count_migrations_with_files() {
        let dir = std::env::temp_dir().join("statecraft_test_count_migrations");
        let _ = std::fs::create_dir_all(&dir);
        // Clean any leftover files
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        std::fs::write(dir.join("0001_first.sql"), "-- UP\n-- DOWN\n").unwrap();
        std::fs::write(dir.join("0002_second.sql"), "-- UP\n-- DOWN\n").unwrap();
        std::fs::write(dir.join("readme.txt"), "ignore me").unwrap();
        assert_eq!(count_migrations(&dir), 2);
        // Cleanup
        let _ = std::fs::remove_file(dir.join("0001_first.sql"));
        let _ = std::fs::remove_file(dir.join("0002_second.sql"));
        let _ = std::fs::remove_file(dir.join("readme.txt"));
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn suggest_subcommand_close_typo() {
        assert_eq!(suggest_subcommand("creat"), Some("create"));
        assert_eq!(suggest_subcommand("lst"), Some("list"));
        assert_eq!(suggest_subcommand("staus"), Some("status"));
    }

    #[test]
    fn suggest_subcommand_too_far() {
        assert_eq!(suggest_subcommand("zzzzzzz"), None);
    }

    #[test]
    fn create_migration_file() {
        let dir = std::env::temp_dir().join("statecraft_test_create_migration");
        let _ = std::fs::create_dir_all(&dir);
        // Clean any leftover files
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }

        // Write a first migration to simulate existing state
        std::fs::write(dir.join("0001_init.sql"), "-- UP\n-- DOWN\n").unwrap();

        let next = count_migrations(&dir) + 1;
        assert_eq!(next, 2);

        let filename = format!("{:04}_add_posts.sql", next);
        let content = "-- UP\n\n-- DOWN\n";
        std::fs::write(dir.join(&filename), content).unwrap();

        let written = std::fs::read_to_string(dir.join(&filename)).unwrap();
        assert_eq!(written, content);
        assert_eq!(filename, "0002_add_posts.sql");

        // Cleanup
        let _ = std::fs::remove_file(dir.join("0001_init.sql"));
        let _ = std::fs::remove_file(dir.join(&filename));
        let _ = std::fs::remove_dir(&dir);
    }
}
