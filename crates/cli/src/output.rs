use pylon_kernel::{Diagnostic, Severity};
use serde::Serialize;

// ---------------------------------------------------------------------------
// ANSI color support
// ---------------------------------------------------------------------------

const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const ITALIC: &str = "\x1b[3m";
const RESET: &str = "\x1b[0m";

/// Returns `true` when stderr should receive colored output.
///
/// Respects the `NO_COLOR` standard (<https://no-color.org/>) and treats
/// `TERM=dumb` as a plain terminal.
fn use_color() -> bool {
    std::env::var("NO_COLOR").is_err()
        && std::env::var("TERM")
            .map(|t| t != "dumb")
            .unwrap_or(true)
}

/// Format a single diagnostic with ANSI colors for human-readable output.
fn format_diagnostic(d: &Diagnostic, color: bool) -> String {
    let mut out = String::new();

    if color {
        let severity_color = match d.severity {
            Severity::Error => RED,
            Severity::Warning => YELLOW,
            Severity::Info => CYAN,
        };

        // e.g. "  error[DEV_NO_ENTRY] No entry file provided"
        out.push_str(&format!(
            "  {BOLD}{severity_color}{}{RESET}{DIM}[{}]{RESET} {}\n",
            d.severity, d.code, d.message,
        ));

        if let Some(hint) = &d.hint {
            out.push_str(&format!(
                "    {DIM}{ITALIC}hint: {hint}{RESET}\n",
            ));
        }
    } else {
        out.push_str(&format!(
            "  {}[{}] {}\n",
            d.severity, d.code, d.message,
        ));

        if let Some(hint) = &d.hint {
            out.push_str(&format!("    hint: {hint}\n"));
        }
    }

    out
}

/// Print diagnostics to stdout/stderr.
/// In JSON mode, serializes as a JSON array.
/// In human mode, prints colored, indented diagnostics.
pub fn print_diagnostics(diagnostics: &[Diagnostic], json_mode: bool) {
    if json_mode {
        let json = serde_json::to_string(diagnostics).unwrap_or_else(|_| "[]".into());
        println!("{json}");
    } else {
        let color = use_color();
        for d in diagnostics {
            let formatted = format_diagnostic(d, color);
            match d.severity {
                Severity::Error => eprint!("{formatted}"),
                _ => print!("{formatted}"),
            }
        }
    }
}

/// Print an ad-hoc error message to stderr with optional color.
pub fn print_error(message: &str) {
    if use_color() {
        eprintln!("{BOLD}{RED}error{RESET}: {message}");
    } else {
        eprintln!("error: {message}");
    }
}

/// Print a serde-serializable value as JSON to stdout.
pub fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string(value) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("[error] JSON_SERIALIZE: Failed to serialize output: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_diagnostic_no_color_no_hint() {
        let d = Diagnostic {
            severity: Severity::Error,
            code: "TEST_ERR".into(),
            message: "something broke".into(),
            span: None,
            hint: None,
        };
        let out = format_diagnostic(&d, false);
        assert_eq!(out, "  error[TEST_ERR] something broke\n");
    }

    #[test]
    fn format_diagnostic_no_color_with_hint() {
        let d = Diagnostic {
            severity: Severity::Warning,
            code: "UNUSED".into(),
            message: "field unused".into(),
            span: None,
            hint: Some("remove it".into()),
        };
        let out = format_diagnostic(&d, false);
        assert_eq!(
            out,
            "  warning[UNUSED] field unused\n    hint: remove it\n"
        );
    }

    #[test]
    fn format_diagnostic_color_includes_ansi() {
        let d = Diagnostic {
            severity: Severity::Error,
            code: "E001".into(),
            message: "bad".into(),
            span: None,
            hint: Some("fix it".into()),
        };
        let out = format_diagnostic(&d, true);
        assert!(out.contains(RED));
        assert!(out.contains(BOLD));
        assert!(out.contains(DIM));
        assert!(out.contains(RESET));
        assert!(out.contains("hint: fix it"));
    }

    #[test]
    fn format_diagnostic_info_uses_cyan() {
        let d = Diagnostic {
            severity: Severity::Info,
            code: "INFO".into(),
            message: "note".into(),
            span: None,
            hint: None,
        };
        let out = format_diagnostic(&d, true);
        assert!(out.contains(CYAN));
    }
}
