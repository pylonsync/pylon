use statecraft_core::ExitCode;
use statecraft_plugin::registry::{PluginCategory, PluginMarketplace, PluginMetadata};

use crate::output;

// ---------------------------------------------------------------------------
// ANSI helpers (matching the output module's conventions)
// ---------------------------------------------------------------------------

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

fn use_color() -> bool {
    std::env::var("NO_COLOR").is_err()
        && std::env::var("TERM")
            .map(|t| t != "dumb")
            .unwrap_or(true)
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    // positional[0] == "plugins"
    match positional.get(1).copied() {
        Some("list") | None => run_list(json_mode),
        Some("search") => {
            let query = positional.get(2).copied().unwrap_or("");
            if query.is_empty() {
                output::print_error("plugins search requires a <query> argument");
                return ExitCode::Usage;
            }
            run_search(query, json_mode)
        }
        Some("info") => {
            let name = positional.get(2).copied().unwrap_or("");
            if name.is_empty() {
                output::print_error("plugins info requires a <name> argument");
                return ExitCode::Usage;
            }
            run_info(name, json_mode)
        }
        Some(sub) => {
            output::print_error(&format!("unknown plugins subcommand: \"{sub}\""));
            eprintln!();
            print_plugins_usage();
            ExitCode::Usage
        }
    }
}

// ---------------------------------------------------------------------------
// `statecraft plugins list`
// ---------------------------------------------------------------------------

fn run_list(json_mode: bool) -> ExitCode {
    let mp = seeded_marketplace();

    if json_mode {
        let all = mp.list_all();
        output::print_json(&all);
        return ExitCode::Ok;
    }

    let color = use_color();
    println!();

    for category in PluginCategory::all_ordered() {
        let mut plugins = mp.by_category(category.clone());
        if plugins.is_empty() {
            continue;
        }
        plugins.sort_by(|a, b| a.name.cmp(&b.name));

        if color {
            println!("  {BOLD}{CYAN}{}{RESET}", category.label());
        } else {
            println!("  {}", category.label());
        }

        for p in &plugins {
            print_plugin_row(p, color);
        }
        println!();
    }

    let total = mp.count();
    if color {
        println!("  {DIM}{total} plugins available{RESET}");
    } else {
        println!("  {total} plugins available");
    }
    println!();

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// `statecraft plugins search <query>`
// ---------------------------------------------------------------------------

fn run_search(query: &str, json_mode: bool) -> ExitCode {
    let mp = seeded_marketplace();
    let mut results = mp.search(query);

    if json_mode {
        output::print_json(&results);
        return ExitCode::Ok;
    }

    let color = use_color();
    println!();

    if results.is_empty() {
        if color {
            println!("  {DIM}No plugins matching \"{query}\"{RESET}");
        } else {
            println!("  No plugins matching \"{query}\"");
        }
        println!();
        return ExitCode::Ok;
    }

    results.sort_by(|a, b| a.name.cmp(&b.name));

    if color {
        println!(
            "  {DIM}Results for \"{query}\":{RESET}"
        );
    } else {
        println!("  Results for \"{query}\":");
    }
    println!();

    for p in &results {
        print_plugin_row(p, color);
    }

    println!();
    if color {
        println!("  {DIM}{} plugin(s) found{RESET}", results.len());
    } else {
        println!("  {} plugin(s) found", results.len());
    }
    println!();

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// `statecraft plugins info <name>`
// ---------------------------------------------------------------------------

fn run_info(name: &str, json_mode: bool) -> ExitCode {
    let mp = seeded_marketplace();

    let Some(p) = mp.get(name) else {
        output::print_error(&format!("plugin \"{name}\" not found"));
        return ExitCode::Error;
    };

    if json_mode {
        output::print_json(&p);
        return ExitCode::Ok;
    }

    let color = use_color();
    println!();

    if color {
        println!("  {BOLD}{}{RESET} {DIM}v{}{RESET}", p.name, p.version);
    } else {
        println!("  {} v{}", p.name, p.version);
    }
    println!("  {}", p.description);
    println!();
    println!("  Category:       {}", p.category.label());
    println!("  Author:         {}", p.author);
    println!("  License:        {}", p.license);
    println!("  Compatibility:  {}", p.compatibility);

    if let Some(ref url) = p.homepage {
        println!("  Homepage:       {url}");
    }
    if let Some(ref url) = p.repository {
        println!("  Repository:     {url}");
    }

    if !p.tags.is_empty() {
        println!("  Tags:           {}", p.tags.join(", "));
    }

    println!();

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn seeded_marketplace() -> PluginMarketplace {
    let mp = PluginMarketplace::new();
    mp.seed_builtins();
    mp
}

/// Print a single plugin row: name (left-aligned, padded) + description.
fn print_plugin_row(p: &PluginMetadata, color: bool) {
    if color {
        println!("    {BOLD}{:<20}{RESET} {DIM}{}{RESET}", p.name, p.description);
    } else {
        println!("    {:<20} {}", p.name, p.description);
    }
}

fn print_plugins_usage() {
    eprintln!("Usage:");
    eprintln!("  statecraft plugins              List all available plugins");
    eprintln!("  statecraft plugins list          List all available plugins");
    eprintln!("  statecraft plugins search <q>    Search plugins by name/tag");
    eprintln!("  statecraft plugins info <name>   Show detailed plugin info");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_returns_ok() {
        let code = run_list(false);
        assert_eq!(code.as_i32(), 0);
    }

    #[test]
    fn list_json_returns_ok() {
        let code = run_list(true);
        assert_eq!(code.as_i32(), 0);
    }

    #[test]
    fn search_finds_jwt() {
        let code = run_search("jwt", false);
        assert_eq!(code.as_i32(), 0);
    }

    #[test]
    fn search_no_results() {
        let code = run_search("nonexistent-xyz-plugin", false);
        assert_eq!(code.as_i32(), 0);
    }

    #[test]
    fn info_existing_plugin() {
        let code = run_info("rate-limit", false);
        assert_eq!(code.as_i32(), 0);
    }

    #[test]
    fn info_missing_plugin() {
        let code = run_info("does-not-exist", false);
        assert_eq!(code.as_i32(), 1);
    }

    #[test]
    fn info_json_mode() {
        let code = run_info("jwt", true);
        assert_eq!(code.as_i32(), 0);
    }
}
