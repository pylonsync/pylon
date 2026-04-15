mod bun;
mod client_codegen;
mod commands;
mod manifest;
mod output;

use agentdb_core::ExitCode;

fn main() {
    std::process::exit(run().as_i32());
}

fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json_mode = args.iter().any(|a| a == "--json");

    // Collect positional args (non-flag) for command dispatch.
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    match positional.first().copied() {
        Some("build") => commands::build::run(&args, json_mode),
        Some("cache") => commands::cache::run(&args, json_mode),
        Some("deploy") => commands::deploy::run(&args, json_mode),
        Some("codegen") => {
            if positional.get(1) == Some(&"client") {
                commands::codegen_client::run(&args, json_mode)
            } else {
                commands::codegen::run(&args, json_mode)
            }
        }
        Some("dev") => commands::dev::run(&args, json_mode),
        Some("doctor") => commands::doctor::run(&args, json_mode),
        Some("env") => commands::env::run(&args, json_mode),
        Some("explain") => commands::explain::run(&args, json_mode),
        Some("init") => commands::init::run(&args, json_mode),
        Some("migrate") => commands::migrate::run(&args, json_mode),
        Some("plugins") => commands::plugins::run(&args, json_mode),
        Some("schema") => match positional.get(1).copied() {
            Some("check") => commands::schema::run_check(&args, json_mode),
            Some("diff") => commands::schema::run_diff(&args, json_mode),
            Some("push") => commands::schema::run_push(&args, json_mode),
            Some("inspect") => commands::schema::run_inspect(&args, json_mode),
            Some("history") => commands::schema::run_history(&args, json_mode),
            Some(sub) => {
                output::print_error(&format!("unknown schema subcommand: \"{sub}\""));
                if let Some(suggestion) = suggest_command(sub, &SCHEMA_SUBCOMMANDS) {
                    eprintln!("  Did you mean: {suggestion}?");
                }
                eprintln!();
                print_usage();
                ExitCode::Usage
            }
            None => {
                output::print_error("schema requires a subcommand");
                eprintln!();
                print_usage();
                ExitCode::Usage
            }
        },
        Some("seed") => commands::seed::run(&args, json_mode),
        Some("version") => commands::version::run(json_mode),
        Some(cmd) => {
            output::print_error(&format!("unknown command: \"{cmd}\""));
            if let Some(suggestion) = suggest_command(cmd, &TOP_LEVEL_COMMANDS) {
                eprintln!("  Did you mean: {suggestion}?");
            }
            eprintln!();
            print_usage();
            ExitCode::Usage
        }
        None => {
            if args.iter().any(|a| a == "--version") {
                commands::version::run(json_mode)
            } else if args.iter().any(|a| a == "--help") {
                print_usage();
                ExitCode::Ok
            } else {
                print_usage();
                ExitCode::Ok
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Known commands for did-you-mean suggestions
// ---------------------------------------------------------------------------

const TOP_LEVEL_COMMANDS: [&str; 15] = [
    "build", "cache", "codegen", "deploy", "dev", "doctor", "env", "explain",
    "init", "migrate", "plugins", "schema", "seed", "version", "help",
];

const SCHEMA_SUBCOMMANDS: [&str; 5] = [
    "check", "diff", "push", "inspect", "history",
];

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

fn print_usage() {
    println!("agentdb -- AI-native framework for web/mobile apps");
    println!();
    println!("Commands:");
    println!("  dev [app.ts]              Start dev server with hot reload");
    println!("  init                      Initialize a new project");
    println!("  build                     Build for production");
    println!("  deploy                    Deploy to production");
    println!("  cache                     Run standalone cache server");
    println!();
    println!("  schema check              Validate schema");
    println!("  schema diff               Show schema changes");
    println!("  schema push               Push schema to database");
    println!("  schema inspect            Inspect live database schema");
    println!("  schema history            Show migration history");
    println!();
    println!("  migrate create <name>     Create a new migration file");
    println!("  migrate list              List all migrations");
    println!("  migrate status            Show migration state");
    println!();
    println!("  codegen                   Generate manifest from TypeScript");
    println!("  codegen client            Generate typed client SDK");
    println!("  seed                      Seed database from JSON file");
    println!();
    println!("  plugins                   List available plugins");
    println!("  plugins search <query>    Search plugins by name/tag");
    println!("  plugins info <name>       Show detailed plugin info");
    println!();
    println!("  doctor                    Check development environment");
    println!("  env                       Show environment variable reference");
    println!("  explain <code>            Explain an error code");
    println!("  version                   Show version");
    println!();
    println!("Options:");
    println!("  --json                    Output as JSON");
    println!("  --help                    Show this message");
}

// ---------------------------------------------------------------------------
// Levenshtein distance and command suggestion
// ---------------------------------------------------------------------------

/// Compute the Levenshtein edit distance between two strings.
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

/// Return the closest matching command if the edit distance is <= 3.
fn suggest_command<'a>(input: &str, commands: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for &cmd in commands {
        let dist = levenshtein(input, cmd);
        if dist <= 3 {
            if best.map_or(true, |(_, d)| dist < d) {
                best = Some((cmd, dist));
            }
        }
    }
    best.map(|(cmd, _)| cmd)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Levenshtein --

    #[test]
    fn levenshtein_identical() {
        assert_eq!(levenshtein("schema", "schema"), 0);
    }

    #[test]
    fn levenshtein_empty_strings() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", "xyz"), 3);
    }

    #[test]
    fn levenshtein_single_edit() {
        assert_eq!(levenshtein("schema", "schem"), 1); // deletion
        assert_eq!(levenshtein("schema", "schemaa"), 1); // insertion
        assert_eq!(levenshtein("schema", "schexa"), 1); // substitution
    }

    #[test]
    fn levenshtein_multiple_edits() {
        assert_eq!(levenshtein("build", "bild"), 1);
        assert_eq!(levenshtein("deploy", "deplo"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn levenshtein_completely_different() {
        assert_eq!(levenshtein("abc", "xyz"), 3);
    }

    // -- suggest_command --

    #[test]
    fn suggest_exact_match() {
        assert_eq!(
            suggest_command("schema", &TOP_LEVEL_COMMANDS),
            Some("schema"),
        );
    }

    #[test]
    fn suggest_close_typo() {
        assert_eq!(
            suggest_command("schem", &TOP_LEVEL_COMMANDS),
            Some("schema"),
        );
        assert_eq!(
            suggest_command("biuld", &TOP_LEVEL_COMMANDS),
            Some("build"),
        );
        assert_eq!(
            suggest_command("chekc", &SCHEMA_SUBCOMMANDS),
            Some("check"),
        );
    }

    #[test]
    fn suggest_nothing_when_too_far() {
        assert_eq!(
            suggest_command("zzzzzzz", &TOP_LEVEL_COMMANDS),
            None,
        );
    }

    #[test]
    fn suggest_picks_closest() {
        // "de" -> "dev" is closer than "deploy"
        assert_eq!(
            suggest_command("de", &["dev", "deploy"]),
            Some("dev"),
        );
    }

    #[test]
    fn suggest_schema_subcommand() {
        assert_eq!(
            suggest_command("pus", &SCHEMA_SUBCOMMANDS),
            Some("push"),
        );
        assert_eq!(
            suggest_command("insepct", &SCHEMA_SUBCOMMANDS),
            Some("inspect"),
        );
        assert_eq!(
            suggest_command("histry", &SCHEMA_SUBCOMMANDS),
            Some("history"),
        );
    }

    #[test]
    fn suggest_migrate_command() {
        assert_eq!(
            suggest_command("migrat", &TOP_LEVEL_COMMANDS),
            Some("migrate"),
        );
    }
}
