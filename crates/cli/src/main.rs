mod artifact;
mod bun;
mod client_codegen;
mod cloud_client;
mod commands;
mod manifest;
mod output;
mod project_context;
mod studio_config;
mod swift_codegen;

use pylon_kernel::ExitCode;

fn main() {
    init_tracing();
    std::process::exit(run().as_i32());
}

/// Initialize structured logging.
///
/// Reads the log level from `RUST_LOG` (e.g. `pylon=debug,info`).
/// Defaults to `info`. Output goes to stderr so logs and JSON output
/// don't intermix on stdout.
///
/// Format is controlled by `PYLON_LOG_FORMAT`:
/// - unset or `pretty` → human-readable compact output (dev default)
/// - `json` → one JSON object per line (Datadog/Axiom/Sentry ingestible)
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    // Default to info, but quiet down chatty third-party crates that
    // emit per-event INFO (Loro's `export: Diagnosing EncodedBlock`
    // fires on every CRDT broadcast and drowns the access-log signal
    // in pylon-cloud's Fly stream). Operators can still re-enable
    // for debugging via RUST_LOG="loro_internal=info,pylon=debug,info".
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,loro_internal=warn"));
    let format = std::env::var("PYLON_LOG_FORMAT").unwrap_or_default();
    let builder = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false);
    if format.eq_ignore_ascii_case("json") {
        let _ = builder.json().try_init();
    } else {
        let _ = builder.compact().try_init();
    }
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

    // Universal `--help` / `-h` short-circuit. Without this, subcommands
    // like `pylon db backup --help` would dispatch through to the
    // command handler, which then sees `--help` as a no-op flag and
    // happily executes the command — `db backup` actually fired the
    // cloud API and got `INVALID_ARGS` back. Help must NEVER cause a
    // side effect.
    //
    // Per-command help printers (logs, etc.) handle their own --help
    // before reaching this point — those are still reachable when the
    // command intercepts the flag itself with richer copy. This is
    // the safety net for everything else.
    if args.iter().any(|a| a == "--help" || a == "-h") {
        // Commands that own a richer help printer get to handle it
        // themselves. Everyone else falls back to the top-level usage.
        match positional.first().copied() {
            Some("logs") => return commands::cloud_logs::run(&args, json_mode),
            // Commands with focused, flag-level help print it here. Help
            // must never cause a side effect, so this only ever prints.
            Some(cmd) if print_command_help(cmd) => return ExitCode::Ok,
            _ => {
                print_usage();
                return ExitCode::Ok;
            }
        }
    }

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
        Some("diagnostics") => commands::diagnostics::run(&args, json_mode),
        Some("doctor") => commands::doctor::run(&args, json_mode),
        Some("env") => commands::env::run(&args, json_mode),
        Some("explain") => commands::explain::run(&args, json_mode),
        Some("fn") => commands::cloud_fn::run(&args, json_mode),
        Some("init") => commands::init::run(&args, json_mode),
        Some("link") => commands::link::run(&args, json_mode),
        Some("lint") => commands::lint::run(&args, json_mode),
        Some("mcp") => commands::mcp::run(&args, json_mode),
        Some("policy") => commands::policy_test::run(&args, json_mode),
        Some("verify") => commands::verify::run(&args, json_mode),
        Some("data") => commands::cloud_data::run(&args, json_mode),
        Some("db") => commands::cloud_db::run(&args, json_mode),
        Some("deployments") => commands::cloud_deployments::run(&args, json_mode),
        Some("domains") => commands::cloud_domains::run(&args, json_mode),
        Some("login") => commands::login::run(&args, json_mode),
        Some("logout") => commands::login::run_logout(&args, json_mode),
        Some("logs") => commands::cloud_logs::run(&args, json_mode),
        Some("members") => commands::cloud_members::run(&args, json_mode),
        Some("migrate") => commands::migrate::run(&args, json_mode),
        Some("projects") => commands::cloud_projects::run(&args, json_mode),
        Some("secrets") => commands::cloud_secrets::run(&args, json_mode),
        Some("status") => commands::cloud_status::run(&args, json_mode),
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
        Some("start") => commands::start::run(&args, json_mode),
        Some("test") => commands::test::run(&args, json_mode),
        Some("test:security") => commands::test_security::run(&args, json_mode),
        Some("backup") => commands::backup::run_backup(&args, json_mode),
        Some("restore") => commands::backup::run_restore(&args, json_mode),
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

const TOP_LEVEL_COMMANDS: [&str; 38] = [
    "backup",
    "build",
    "cache",
    "codegen",
    "data",
    "db",
    "deploy",
    "deployments",
    "dev",
    "diagnostics",
    "doctor",
    "domains",
    "env",
    "explain",
    "fn",
    "init",
    "link",
    "lint",
    "mcp",
    "policy",
    "verify",
    "login",
    "logout",
    "logs",
    "members",
    "migrate",
    "plugins",
    "projects",
    "restore",
    "schema",
    "secrets",
    "seed",
    "start",
    "status",
    "test",
    "test:security",
    "version",
    "help",
];

const SCHEMA_SUBCOMMANDS: [&str; 5] = ["check", "diff", "push", "inspect", "history"];

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

/// Focused, flag-level `--help` for the commands that take flags worth
/// documenting. Returns true if it printed help for `cmd`, false to let
/// the caller fall back to the top-level usage. Prints only — no side
/// effects — so `pylon <cmd> --help` never runs the command.
fn print_command_help(cmd: &str) -> bool {
    match cmd {
        "dev" => {
            println!("pylon dev — start the dev server with hot reload");
            println!();
            println!("Usage:");
            println!("  pylon dev [app.ts] [--port <n>] [--json]");
            println!();
            println!("Serves your SSR frontend and API from one port. Regenerates the manifest");
            println!("and typed client on every save and reloads on change. Loads .env and");
            println!(".env.local automatically (walking up to the workspace root).");
            println!();
            println!("Arguments:");
            println!("  app.ts            Manifest entry file (default: ./app.ts)");
            println!();
            println!("Options:");
            println!("  -p, --port <n>    Port to listen on (default: 4321)");
            println!("  --once            Build once and exit, no watcher (handy in CI)");
            println!("  --json            Emit machine-readable JSON build/watch events");
            println!("  -h, --help        Show this help");
            println!();
            println!("Environment:");
            println!("  PYLON_DB_PATH              SQLite dev database (default: .pylon/dev.db)");
            println!("  PYLON_FILES_DIR            Local file-storage directory");
            println!("  PYLON_CORS_ORIGIN          Allowed CORS origin for the API");
            println!("  PYLON_FRONTEND_DEV_PROXY   Proxy non-API GETs to an external frontend dev server");
            println!("  PYLON_RATE_LIMIT_MAX       Per-IP request budget");
            println!("  PYLON_FN_RATE_LIMIT_MAX    Per-IP function-call budget");
            println!();
            println!("Examples:");
            println!("  pylon dev");
            println!("  pylon dev --port 3000");
            println!("  pylon dev app.ts --json");
            true
        }
        "start" => {
            println!("pylon start — run the production server (no watcher)");
            println!();
            println!("Usage:");
            println!("  pylon start [app.ts] [--port <n>]");
            println!();
            println!("The same server as `dev`, without the file watcher or codegen. Expects an");
            println!("already-built manifest + client — run `pylon build` first.");
            println!();
            println!("Options:");
            println!("  -p, --port <n>    Port to listen on (default: 4321)");
            println!("  -h, --help        Show this help");
            println!();
            println!("Environment:");
            println!("  PYLON_DB_PATH     SQLite database file");
            println!("  PYLON_FILES_DIR   Local file-storage directory");
            println!("  PYLON_CORS_ORIGIN Allowed CORS origin for the API");
            true
        }
        "build" => {
            println!("pylon build — build for production");
            println!();
            println!("Usage:");
            println!("  pylon build [app.ts] [--out <dir>] [--json]");
            println!();
            println!("Compiles the manifest, generates the typed client, and builds the SSR");
            println!("frontend bundle. Run before `pylon start` or `pylon deploy`.");
            println!();
            println!("Options:");
            println!("  --out <dir>          Output directory (default: .pylon)");
            println!("  --manifest-out <f>   Manifest JSON output path");
            println!("  --client-out <f>     Generated client output path");
            println!("  --no-client          Skip typed-client codegen");
            println!("  --no-static          Skip the frontend bundle");
            println!("  --json               Machine-readable output");
            println!("  -h, --help           Show this help");
            true
        }
        "init" => {
            println!("pylon init — scaffold a new project");
            println!();
            println!("Usage:");
            println!("  pylon init [--frontend <f>] [--template <t>] [--yes]");
            println!();
            println!("Options:");
            println!("  --frontend <f>       nextjs | react | tanstack | none");
            println!("  --template <t>       Starter template");
            println!("  --bun                Use bun as the package manager");
            println!("  --yes, --no-prompt   Accept defaults, skip interactive prompts");
            println!("  --overwrite          Allow writing into a non-empty directory");
            println!("  -h, --help           Show this help");
            true
        }
        "deploy" => {
            println!("pylon deploy — deploy your app");
            println!();
            println!("Usage:");
            println!("  pylon deploy [--target <target>] [--project <slug>]");
            println!();
            println!("Targets:");
            println!("  cloud (default)      Pylon Cloud");
            println!("  docker | fly | compose | workers | systemd | manifest");
            println!();
            println!("Options:");
            println!("  --target <t>         Deploy target (default: cloud)");
            println!("  --project <slug>     Cloud project to deploy to");
            println!("  --out <dir>          Output directory (file-emitting targets)");
            println!("  -h, --help           Show this help");
            true
        }
        _ => false,
    }
}

fn print_usage() {
    println!("pylon -- AI-native framework for web/mobile apps");
    println!();
    println!("Commands:");
    println!("  dev [app.ts]              Start dev server with hot reload");
    println!("  start [app.ts]            Start production server (no watcher)");
    println!("  init                      Initialize a new project");
    println!("  build                     Build for production");
    println!(
        "  deploy                    Deploy to Pylon Cloud (or --target docker|fly|compose|workers|systemd|manifest)"
    );
    println!("  cache                     Run standalone cache server");
    println!("  test [filter]             Run *.test.ts files against an in-memory Pylon");
    println!("  test:security             Adversarial security probe against a running app");
    println!();
    println!("  login                     Authenticate against Pylon Cloud");
    println!("  logout                    Remove stored Pylon Cloud credentials");
    println!("  link                      Connect this project to a GitHub repo for auto-deploy");
    println!("  projects [list|use|current]   List / set / show current cloud project");
    println!("  secrets  [list|set|rm|import] Manage project secrets");
    println!("  logs tail                 Tail the project's request log");
    println!("  domains  [list|add|verify|rm] Manage custom domains");
    println!("  db       [list|backup|restore] Database snapshots");
    println!("  data     [entities|list|get] Browse entity rows from the shell");
    println!("  deployments [list|logs|rollback] List, tail build logs, roll back");
    println!("  members  [list|invite]    Org members");
    println!("  fn <name> [k=v ...]       Call any Pylon Cloud function by name");
    println!("  status                    One-glance project health");
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
    println!("  diagnostics               Read the dev server's SSR cache/render diagnostics");
    println!("  doctor                    Check development environment");
    println!("  env                       Show environment variable reference");
    println!(
        "  explain <code>            Explain an error code
  mcp [--url <base>]        Serve this app to agents over MCP (stdio)
  policy test <expr>        Dry-run a policy expression (--auth k=v --row json)
  verify [--url <base>]     Boot (or target) the app and verify routes + assets serve"
    );
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
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
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
        assert_eq!(suggest_command("biuld", &TOP_LEVEL_COMMANDS), Some("build"),);
        assert_eq!(suggest_command("chekc", &SCHEMA_SUBCOMMANDS), Some("check"),);
    }

    #[test]
    fn suggest_nothing_when_too_far() {
        assert_eq!(suggest_command("zzzzzzz", &TOP_LEVEL_COMMANDS), None,);
    }

    #[test]
    fn suggest_picks_closest() {
        // "de" -> "dev" is closer than "deploy"
        assert_eq!(suggest_command("de", &["dev", "deploy"]), Some("dev"),);
    }

    #[test]
    fn suggest_schema_subcommand() {
        assert_eq!(suggest_command("pus", &SCHEMA_SUBCOMMANDS), Some("push"),);
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
