use pylon_kernel::ExitCode;
use serde::Serialize;

use crate::output;

// ---------------------------------------------------------------------------
// Environment variable definitions
// ---------------------------------------------------------------------------

struct EnvVarDef {
    name: &'static str,
    description: &'static str,
    default: Option<&'static str>,
    sensitive: bool,
}

const ENV_VARS: &[EnvVarDef] = &[
    EnvVarDef {
        name: "PYLON_ADMIN_TOKEN",
        description: "Admin auth token for protected endpoints",
        default: None,
        sensitive: true,
    },
    EnvVarDef {
        name: "PYLON_RATE_LIMIT_MAX",
        description: "Max requests per window",
        default: Some("100"),
        sensitive: false,
    },
    EnvVarDef {
        name: "PYLON_RATE_LIMIT_WINDOW",
        description: "Rate limit window in seconds",
        default: Some("60"),
        sensitive: false,
    },
    EnvVarDef {
        name: "PYLON_OAUTH_GOOGLE_CLIENT_ID",
        description: "Google OAuth client ID",
        default: None,
        sensitive: false,
    },
    EnvVarDef {
        name: "PYLON_OAUTH_GOOGLE_CLIENT_SECRET",
        description: "Google OAuth client secret",
        default: None,
        sensitive: true,
    },
    EnvVarDef {
        name: "PYLON_OAUTH_GITHUB_CLIENT_ID",
        description: "GitHub OAuth client ID",
        default: None,
        sensitive: false,
    },
    EnvVarDef {
        name: "PYLON_OAUTH_GITHUB_CLIENT_SECRET",
        description: "GitHub OAuth client secret",
        default: None,
        sensitive: true,
    },
    EnvVarDef {
        name: "NO_COLOR",
        description: "Disable colored output",
        default: None,
        sensitive: false,
    },
    EnvVarDef {
        name: "TERM",
        description: "Terminal type (dumb = no color)",
        default: None,
        sensitive: false,
    },
];

// ---------------------------------------------------------------------------
// JSON output shape
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct JsonEnvVar {
    name: String,
    description: String,
    set: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<String>,
}

// ---------------------------------------------------------------------------
// ANSI helpers
// ---------------------------------------------------------------------------

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

fn use_color() -> bool {
    std::env::var("NO_COLOR").is_err()
        && std::env::var("TERM")
            .map(|t| t != "dumb")
            .unwrap_or(true)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(_args: &[String], json_mode: bool) -> ExitCode {
    if json_mode {
        let vars: Vec<JsonEnvVar> = ENV_VARS
            .iter()
            .map(|def| {
                let value = std::env::var(def.name).ok();
                let masked_value = value.as_ref().map(|v| {
                    if def.sensitive {
                        "***".to_string()
                    } else {
                        v.clone()
                    }
                });
                JsonEnvVar {
                    name: def.name.into(),
                    description: def.description.into(),
                    set: value.is_some(),
                    value: masked_value,
                    default: def.default.map(|s| s.into()),
                }
            })
            .collect();
        output::print_json(&vars);
        return ExitCode::Ok;
    }

    let color = use_color();

    // Find the longest variable name for alignment.
    let max_name_len = ENV_VARS.iter().map(|v| v.name.len()).max().unwrap_or(0);

    println!();
    if color {
        println!("{BOLD}pylon environment variables{RESET}");
    } else {
        println!("pylon environment variables");
    }
    println!();

    for def in ENV_VARS {
        let padding = " ".repeat(max_name_len - def.name.len() + 2);
        if color {
            println!(
                "  {BOLD}{}{RESET}{padding}{DIM}{}{RESET}",
                def.name, def.description,
            );
        } else {
            println!("  {}{padding}{}", def.name, def.description);
        }
        if let Some(default) = def.default {
            if color {
                println!(
                    "  {}{padding}{DIM}default: {default}{RESET}",
                    " ".repeat(def.name.len()),
                );
            } else {
                println!(
                    "  {}{padding}default: {default}",
                    " ".repeat(def.name.len()),
                );
            }
        }
    }

    println!();
    if color {
        println!("{BOLD}Current values:{RESET}");
    } else {
        println!("Current values:");
    }
    println!();

    for def in ENV_VARS {
        let padding = " ".repeat(max_name_len - def.name.len() + 2);
        let value = std::env::var(def.name).ok();

        let status = match (&value, def.default) {
            (Some(_v), _) if def.sensitive => {
                if color {
                    format!("{GREEN}[set]{RESET} {DIM}(value hidden){RESET}")
                } else {
                    "[set] (value hidden)".into()
                }
            }
            (Some(v), _) => {
                if color {
                    format!("{GREEN}[set]{RESET} {DIM}{v}{RESET}")
                } else {
                    format!("[set] {v}")
                }
            }
            (None, Some(default)) => {
                if color {
                    format!("{YELLOW}[not set]{RESET} {DIM}(using default: {default}){RESET}")
                } else {
                    format!("[not set] (using default: {default})")
                }
            }
            (None, None) => {
                if color {
                    format!("{YELLOW}[not set]{RESET}")
                } else {
                    "[not set]".into()
                }
            }
        };

        println!("  {}{padding}{status}", def.name);
    }

    println!();

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_var_definitions_are_non_empty() {
        assert!(!ENV_VARS.is_empty());
    }

    #[test]
    fn all_names_are_uppercase() {
        for def in ENV_VARS {
            // Allow TERM and NO_COLOR which are standard.
            assert_eq!(
                def.name,
                def.name.to_uppercase(),
                "env var name should be uppercase: {}",
                def.name,
            );
        }
    }

    #[test]
    fn sensitive_vars_contain_token_or_secret() {
        for def in ENV_VARS {
            if def.sensitive {
                let upper = def.name.to_uppercase();
                assert!(
                    upper.contains("TOKEN") || upper.contains("SECRET"),
                    "sensitive var {} should contain TOKEN or SECRET in its name",
                    def.name,
                );
            }
        }
    }

    #[test]
    fn no_duplicate_names() {
        let mut seen = std::collections::HashSet::new();
        for def in ENV_VARS {
            assert!(
                seen.insert(def.name),
                "duplicate env var definition: {}",
                def.name,
            );
        }
    }
}
