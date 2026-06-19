//! Argument-parsing helpers shared across commands.
//!
//! The reason this exists: `dev` and `start` both want to accept a
//! positional entry file alongside named flags like `--port 3001`.
//! Naive filtering (`.filter(|a| !a.starts_with('-'))`) keeps `3001`
//! as a positional — the value following a flag survives the filter
//! because it doesn't itself start with `-`. The downstream code
//! then treats `3001` as an entry-file path and reports
//! "Entry file not found: 3001" on every `pylon dev --port 3001`.
//!
//! `collect_positional` solves it by knowing which flags consume the
//! next arg, and skipping both indices on every match.

/// Flags that take a value. The arg directly after one of these is
/// the flag's value, NOT a positional. Keep this list in sync with
/// the flags each command accepts.
const VALUE_FLAGS: &[&str] = &[
    "--port",
    "--manifest",
    "--entry",
    "--config",
    "--out",
    "--host",
    "--region",
    "--app",
    "--from",
    "--to",
    "--name",
    "--cwd",
];

/// Short flags that consume the next arg as their value. `-p` is the
/// short form of `--port` (dev/start) and `--project` (cloud commands);
/// either way the following token is its value, not a positional.
const VALUE_SHORT_FLAGS: &[&str] = &["-p"];

/// Collect positional args (entry file path, etc.) from an argv slice,
/// skipping the leading subcommand name + any `--flag value` pairs.
pub fn collect_positional<'a>(args: &'a [String], subcommand: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == subcommand {
            i += 1;
            continue;
        }
        if a.starts_with("--") {
            // `--flag=value` is self-contained; `--flag` followed by
            // a value-flag is a pair we skip over.
            if !a.contains('=') && VALUE_FLAGS.contains(&a) && i + 1 < args.len() {
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if a.starts_with('-') {
            // Short flags. Value-bearing ones (e.g. `-p 3000`) consume the
            // next token as their value, so it isn't mistaken for a
            // positional. `-p=3000` is self-contained.
            if !a.contains('=') && VALUE_SHORT_FLAGS.contains(&a) && i + 1 < args.len() {
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        out.push(a);
        i += 1;
    }
    out
}

/// Resolve the listen port for dev/start commands.
///
/// Precedence is intentionally explicit: a command-line flag beats
/// `PYLON_PORT`, and `PYLON_PORT` beats the command default.
pub fn parse_port(args: &[String], default: u16) -> u16 {
    parse_port_with_env(args, default, std::env::var("PYLON_PORT").ok())
}

fn parse_port_with_env(args: &[String], default: u16, env_value: Option<String>) -> u16 {
    parse_port_arg(args)
        .or_else(|| env_value.and_then(|v| v.parse().ok()))
        .unwrap_or(default)
}

fn parse_port_arg(args: &[String]) -> Option<u16> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--port" || a == "-p" {
            return args.get(i + 1).and_then(|v| v.parse().ok());
        }
        if let Some(v) = a.strip_prefix("--port=") {
            return v.parse().ok();
        }
        if let Some(v) = a.strip_prefix("-p=") {
            return v.parse().ok();
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn drops_subcommand() {
        let args = s(&["dev"]);
        assert_eq!(collect_positional(&args, "dev"), Vec::<&str>::new());
    }

    #[test]
    fn keeps_positional_entry() {
        let args = s(&["dev", "schema.ts"]);
        assert_eq!(collect_positional(&args, "dev"), vec!["schema.ts"]);
    }

    /// The original bug: --port 3001 left 3001 in the positional list.
    #[test]
    fn skips_port_value() {
        let args = s(&["dev", "--port", "3001"]);
        assert_eq!(collect_positional(&args, "dev"), Vec::<&str>::new());
    }

    #[test]
    fn skips_port_value_with_entry_after() {
        let args = s(&["dev", "--port", "3001", "schema.ts"]);
        assert_eq!(collect_positional(&args, "dev"), vec!["schema.ts"]);
    }

    #[test]
    fn skips_port_value_with_entry_before() {
        let args = s(&["dev", "schema.ts", "--port", "3001"]);
        assert_eq!(collect_positional(&args, "dev"), vec!["schema.ts"]);
    }

    /// `-p 3000` (short --port) must not leave 3000 as a positional —
    /// otherwise dev treats it as the entry file ("not found: 3000").
    #[test]
    fn skips_short_port_value() {
        let args = s(&["dev", "-p", "3000"]);
        assert_eq!(collect_positional(&args, "dev"), Vec::<&str>::new());
    }

    #[test]
    fn skips_short_port_value_with_entry() {
        let args = s(&["dev", "-p", "3000", "app.ts"]);
        assert_eq!(collect_positional(&args, "dev"), vec!["app.ts"]);
    }

    #[test]
    fn handles_inline_value_form() {
        let args = s(&["dev", "--port=3001", "schema.ts"]);
        assert_eq!(collect_positional(&args, "dev"), vec!["schema.ts"]);
    }

    #[test]
    fn handles_boolean_flag_followed_by_positional() {
        // --json is a boolean flag; the next arg is a positional.
        let args = s(&["dev", "--json", "schema.ts"]);
        assert_eq!(collect_positional(&args, "dev"), vec!["schema.ts"]);
    }

    #[test]
    fn handles_multiple_value_flags() {
        let args = s(&["start", "--port", "3001", "--host", "0.0.0.0", "app.ts"]);
        assert_eq!(collect_positional(&args, "start"), vec!["app.ts"]);
    }

    #[test]
    fn parse_port_uses_default_without_flag_or_env() {
        let args = s(&["start", "app.ts"]);
        assert_eq!(parse_port_with_env(&args, 4321, None), 4321);
    }

    #[test]
    fn parse_port_uses_env_when_flag_missing() {
        let args = s(&["start", "app.ts"]);
        assert_eq!(parse_port_with_env(&args, 4321, Some("8080".into())), 8080);
    }

    #[test]
    fn parse_port_flag_beats_env() {
        let args = s(&["start", "--port", "3000", "app.ts"]);
        assert_eq!(parse_port_with_env(&args, 4321, Some("8080".into())), 3000);
    }

    #[test]
    fn parse_port_short_flag_beats_env() {
        let args = s(&["start", "-p", "3000", "app.ts"]);
        assert_eq!(parse_port_with_env(&args, 4321, Some("8080".into())), 3000);
    }

    #[test]
    fn parse_port_handles_inline_long_flag() {
        let args = s(&["start", "--port=3000", "app.ts"]);
        assert_eq!(parse_port_with_env(&args, 4321, Some("8080".into())), 3000);
    }

    #[test]
    fn parse_port_handles_inline_short_flag() {
        let args = s(&["start", "-p=3000", "app.ts"]);
        assert_eq!(parse_port_with_env(&args, 4321, Some("8080".into())), 3000);
    }

    #[test]
    fn parse_port_ignores_invalid_env() {
        let args = s(&["start", "app.ts"]);
        assert_eq!(parse_port_with_env(&args, 4321, Some("nope".into())), 4321);
    }
}
