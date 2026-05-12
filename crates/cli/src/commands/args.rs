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
            // Short flags. Pylon's CLI doesn't use value-bearing
            // short flags today; if it ever does, extend this branch.
            i += 1;
            continue;
        }
        out.push(a);
        i += 1;
    }
    out
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
}
