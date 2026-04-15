use std::path::Path;

use agentdb_core::ExitCode;

use crate::output;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MIGRATIONS_DIR: &str = "migrations";

const MIGRATE_SUBCOMMANDS: [&str; 3] = ["create", "list", "status"];

// ---------------------------------------------------------------------------
// Entry point — dispatches to subcommands
// ---------------------------------------------------------------------------

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    match positional.get(1).copied() {
        Some("create") => run_create(&positional, json_mode),
        Some("list") => run_list(json_mode),
        Some("status") => run_status(json_mode),
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
            eprintln!("  Usage: agentdb migrate create <name>");
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
                // A future enhancement can check _agentdb_migrations table.
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
    eprintln!("Usage: agentdb migrate <subcommand>");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  create <name>   Create a new migration file");
    eprintln!("  list            List all migrations and their status");
    eprintln!("  status          Show current migration state");
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
        let dir = std::env::temp_dir().join("agentdb_test_empty_migrations");
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
        let dir = std::env::temp_dir().join("agentdb_test_count_migrations");
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
        let dir = std::env::temp_dir().join("agentdb_test_create_migration");
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
