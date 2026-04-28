use std::io::{IsTerminal, Write};
use std::path::Path;

use pylon_kernel::{Diagnostic, ExitCode, Severity};
use serde::Serialize;

use crate::bun::run_bun_codegen;
use crate::client_codegen::generate_client_ts;
use crate::manifest::parse_manifest;
use crate::output::{print_diagnostics, print_json};

// Vendored API templates — survive `cargo package`.
const TEMPLATE_BASIC_APP: &str = include_str!("../../templates/basic/app.ts");
const TEMPLATE_BASIC_TSCONFIG: &str = include_str!("../../templates/basic/tsconfig.json");
const SDK_SOURCE: &str = include_str!("../../embedded/sdk-index.ts");

// ---- Frontend templates -----------------------------------------------------

// `cn` helper — shipped inline because shadcn-installed components
// import `@/lib/utils`. The components themselves are added via the
// shadcn CLI in a post-scaffold step (see `run_shadcn_add`).
const SHARED_UTILS: &str = include_str!("../../templates/_shared-frontend/lib/utils.ts");

/// Components the shadcn CLI installs into every new project. Tweak by
/// editing this list; users can add more later via
/// `bunx shadcn@latest add <name>`.
const SHADCN_COMPONENTS: &[&str] = &["button", "input", "label", "card"];

// Next.js
const NEXTJS_PACKAGE_JSON: &str = include_str!("../../templates/nextjs/package.json");
const NEXTJS_TSCONFIG: &str = include_str!("../../templates/nextjs/tsconfig.json");
const NEXTJS_NEXT_CONFIG: &str = include_str!("../../templates/nextjs/next.config.js");
const NEXTJS_NEXT_ENV: &str = include_str!("../../templates/nextjs/next-env.d.ts");
const NEXTJS_POSTCSS: &str = include_str!("../../templates/nextjs/postcss.config.mjs");
const NEXTJS_COMPONENTS_JSON: &str = include_str!("../../templates/nextjs/components.json");
const NEXTJS_GLOBALS_CSS: &str = include_str!("../../templates/nextjs/app/globals.css");
const NEXTJS_LIB_PYLON: &str = include_str!("../../templates/nextjs/lib/pylon.ts");
const NEXTJS_PROXY: &str = include_str!("../../templates/nextjs/proxy.ts");
const NEXTJS_LAYOUT: &str = include_str!("../../templates/nextjs/app/layout.tsx");
const NEXTJS_PROVIDERS: &str = include_str!("../../templates/nextjs/app/providers.tsx");
const NEXTJS_PAGE: &str = include_str!("../../templates/nextjs/app/page.tsx");
const NEXTJS_LOGIN_PAGE: &str = include_str!("../../templates/nextjs/app/login/page.tsx");
const NEXTJS_LOGIN_FORM: &str = include_str!("../../templates/nextjs/app/login/form.tsx");
const NEXTJS_LOGIN_ACTIONS: &str = include_str!("../../templates/nextjs/app/login/actions.ts");
const NEXTJS_DASHBOARD_LAYOUT: &str =
    include_str!("../../templates/nextjs/app/dashboard/layout.tsx");
const NEXTJS_DASHBOARD_PAGE: &str = include_str!("../../templates/nextjs/app/dashboard/page.tsx");

// Vite + React
const REACT_PACKAGE_JSON: &str = include_str!("../../templates/react/package.json");
const REACT_TSCONFIG: &str = include_str!("../../templates/react/tsconfig.json");
const REACT_VITE_CONFIG: &str = include_str!("../../templates/react/vite.config.ts");
const REACT_INDEX_HTML: &str = include_str!("../../templates/react/index.html");
const REACT_MAIN: &str = include_str!("../../templates/react/src/main.tsx");
const REACT_APP: &str = include_str!("../../templates/react/src/App.tsx");
const REACT_LIB_PYLON: &str = include_str!("../../templates/react/src/lib/pylon.ts");
const REACT_LOGIN: &str = include_str!("../../templates/react/src/Login.tsx");
const REACT_DASHBOARD: &str = include_str!("../../templates/react/src/Dashboard.tsx");

// TanStack Start
const TANSTACK_PACKAGE_JSON: &str = include_str!("../../templates/tanstack/package.json");
const TANSTACK_TSCONFIG: &str = include_str!("../../templates/tanstack/tsconfig.json");
const TANSTACK_APP_CONFIG: &str = include_str!("../../templates/tanstack/app.config.ts");
const TANSTACK_ROUTER: &str = include_str!("../../templates/tanstack/app/router.tsx");
const TANSTACK_CLIENT: &str = include_str!("../../templates/tanstack/app/client.tsx");
const TANSTACK_SSR: &str = include_str!("../../templates/tanstack/app/ssr.tsx");
const TANSTACK_ROOT_ROUTE: &str = include_str!("../../templates/tanstack/app/routes/__root.tsx");
const TANSTACK_INDEX_ROUTE: &str = include_str!("../../templates/tanstack/app/routes/index.tsx");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frontend {
    NextJs,
    React,
    TanStack,
    None,
}

impl Frontend {
    fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "next" | "nextjs" | "next.js" => Some(Self::NextJs),
            "react" | "vite" | "vite-react" => Some(Self::React),
            "tanstack" | "tanstack-start" | "start" => Some(Self::TanStack),
            "none" | "no" | "skip" => Some(Self::None),
            _ => None,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::NextJs => "Next.js",
            Self::React => "React + Vite",
            Self::TanStack => "TanStack Start",
            Self::None => "no frontend",
        }
    }
}

#[derive(Serialize)]
struct InitOutput {
    code: &'static str,
    name: String,
    path: String,
    frontend: String,
    files: Vec<String>,
}

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let frontend_arg = args
        .windows(2)
        .find(|w| w[0] == "--frontend")
        .map(|w| w[1].as_str());

    let template_arg = args
        .windows(2)
        .find(|w| w[0] == "--template")
        .map(|w| w[1].as_str())
        .unwrap_or("basic");

    let no_prompt = args
        .iter()
        .any(|a| a == "--no-prompt" || a == "--yes" || a == "-y");

    // Filter positional args: drop the flag NAMES, the flag VALUES, and the
    // dispatcher's own "init" token.
    let flag_consumers: std::collections::HashSet<&str> =
        ["--frontend", "--template"].into_iter().collect();
    let flag_values: std::collections::HashSet<&str> = args
        .windows(2)
        .filter(|w| flag_consumers.contains(w[0].as_str()))
        .map(|w| w[1].as_str())
        .collect();

    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "init" && !flag_values.contains(a.as_str()))
        .map(|s| s.as_str())
        .collect();

    let target_arg = match positional.first() {
        Some(n) => *n,
        None => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "INIT_NO_PATH".into(),
                    message: "No target path provided".into(),
                    span: None,
                    hint: Some(
                        "Usage: pylon init <path> [--frontend nextjs|react|tanstack|none]".into(),
                    ),
                }],
                json_mode,
            );
            return ExitCode::Usage;
        }
    };

    let target = Path::new(target_arg);

    let app_name = match target.file_name().and_then(|n| n.to_str()) {
        Some(n) if !n.is_empty() && !n.starts_with('.') => n.to_string(),
        _ => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "INIT_INVALID_NAME".into(),
                    message: format!(
                        "Could not derive a valid app name from path: \"{}\"",
                        target.display()
                    ),
                    span: None,
                    hint: Some(
                        "The final path segment must be a valid name (non-empty, no leading dot)"
                            .into(),
                    ),
                }],
                json_mode,
            );
            return ExitCode::Usage;
        }
    };

    if template_arg != "basic" {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "INIT_UNKNOWN_TEMPLATE".into(),
                message: format!("Unknown template: \"{template_arg}\""),
                span: None,
                hint: Some("Available templates: basic".into()),
            }],
            json_mode,
        );
        return ExitCode::Usage;
    }

    // Resolve frontend choice. Precedence:
    //   1. --frontend CLI flag (always wins; used by CI / scripts)
    //   2. Interactive prompt (only when stdin is a TTY and not --no-prompt
    //      and not JSON-mode)
    //   3. Default to Next.js
    let frontend = match frontend_arg {
        Some(s) => match Frontend::parse(s) {
            Some(f) => f,
            None => {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Error,
                        code: "INIT_UNKNOWN_FRONTEND".into(),
                        message: format!("Unknown frontend: \"{s}\""),
                        span: None,
                        hint: Some("Available: nextjs, react, tanstack, none".into()),
                    }],
                    json_mode,
                );
                return ExitCode::Usage;
            }
        },
        None => {
            if !json_mode && !no_prompt && std::io::stdin().is_terminal() {
                prompt_frontend()
            } else {
                Frontend::NextJs
            }
        }
    };

    if target.exists() {
        let is_empty = match std::fs::read_dir(target) {
            Ok(mut entries) => entries.next().is_none(),
            Err(_) => false,
        };
        if !is_empty {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "INIT_DIR_EXISTS".into(),
                    message: format!(
                        "Directory \"{}\" already exists and is not empty",
                        target.display()
                    ),
                    span: None,
                    hint: Some("Choose a different path or remove the existing directory".into()),
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
    }

    if let Err(e) = std::fs::create_dir_all(target) {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "INIT_MKDIR_FAILED".into(),
                message: format!("Could not create directory \"{}\": {e}", target.display()),
                span: None,
                hint: None,
            }],
            json_mode,
        );
        return ExitCode::Error;
    }

    let mut written_files: Vec<String> = Vec::new();

    // ---- Workspace root ----------------------------------------------------
    let root_pkg = workspace_root_package_json(&app_name, frontend);
    if let Err(code) = write_file(
        target,
        "package.json",
        &root_pkg,
        json_mode,
        &mut written_files,
    ) {
        return code;
    }
    let gitignore = WORKSPACE_GITIGNORE;
    if let Err(code) = write_file(
        target,
        ".gitignore",
        gitignore,
        json_mode,
        &mut written_files,
    ) {
        return code;
    }
    let readme = workspace_readme(&app_name, frontend);
    if let Err(code) = write_file(target, "README.md", &readme, json_mode, &mut written_files) {
        return code;
    }

    // ---- apps/api ----------------------------------------------------------
    let api_dir = target.join("apps").join("api");
    if let Err(e) = std::fs::create_dir_all(&api_dir) {
        return mkdir_err(&api_dir, e, json_mode);
    }
    let app_ts = TEMPLATE_BASIC_APP.replace("__APP_NAME__", &app_name);
    let api_pkg = api_package_json(&app_name);
    let api_files: &[(&str, &str)] = &[
        ("sdk.ts", SDK_SOURCE),
        ("app.ts", &app_ts),
        ("tsconfig.json", TEMPLATE_BASIC_TSCONFIG),
        ("package.json", &api_pkg),
    ];
    for (name, contents) in api_files {
        if let Err(code) = write_file(&api_dir, name, contents, json_mode, &mut written_files) {
            return code;
        }
    }

    // ---- apps/web (chosen frontend) ---------------------------------------
    let web_dir_for_shadcn = if frontend != Frontend::None {
        let web_dir = target.join("apps").join("web");
        if let Err(e) = std::fs::create_dir_all(&web_dir) {
            return mkdir_err(&web_dir, e, json_mode);
        }
        if let Err(code) =
            write_frontend(frontend, &web_dir, &app_name, json_mode, &mut written_files)
        {
            return code;
        }
        Some(web_dir)
    } else {
        None
    };

    // ---- bun install + shadcn UI components -------------------------------
    // shadcn's CLI needs node_modules in place to resolve its own deps,
    // so we install workspace deps first, then add the components. Both
    // steps degrade to a clear hint on failure — the rest of the project
    // is fully usable; the user just runs the command themselves.
    let mut deps_installed = false;
    if let Some(ref web_dir) = web_dir_for_shadcn {
        deps_installed = run_bun_install(target, json_mode);
        if deps_installed {
            run_shadcn_add(web_dir, &mut written_files, json_mode);
        } else {
            // Without node_modules the shadcn CLI will fail at "Resolving
            // dependencies"; surface a single combined hint instead.
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Warning,
                    code: "INIT_SKIPPED_SHADCN".into(),
                    message: "Skipped shadcn UI install (bun install didn't run)".into(),
                    span: None,
                    hint: Some(format!(
                        "Run inside the project: bun install && cd apps/web && bunx shadcn@latest add {}",
                        SHADCN_COMPONENTS.join(" ")
                    )),
                }],
                json_mode,
            );
        }
    }

    // ---- Codegen the API manifest -----------------------------------------
    let entry_path = api_dir.join("app.ts");
    let manifest_path = api_dir.join("pylon.manifest.json");
    let entry_str = entry_path.to_string_lossy().to_string();
    match run_bun_codegen(&entry_str) {
        Ok(manifest_json) => {
            let contents = format!("{manifest_json}\n");
            if let Err(e) = std::fs::write(&manifest_path, &contents) {
                print_diagnostics(
                    &[Diagnostic {
                        severity: Severity::Warning,
                        code: "INIT_CODEGEN_WRITE_FAILED".into(),
                        message: format!("Files created but could not write manifest: {e}"),
                        span: None,
                        hint: Some("Run 'cd apps/api && bun run codegen' manually".into()),
                    }],
                    json_mode,
                );
            } else {
                written_files.push(rel_path(target, &manifest_path));
            }
            if let Ok(manifest) = parse_manifest(&manifest_json, &entry_str) {
                let client_ts = generate_client_ts(&manifest);
                let client_path = api_dir.join("pylon.client.ts");
                if std::fs::write(&client_path, client_ts).is_ok() {
                    written_files.push(rel_path(target, &client_path));
                }
            }
        }
        Err(diag) => {
            let bun_missing = std::process::Command::new("bun")
                .arg("--version")
                .output()
                .map(|o| !o.status.success())
                .unwrap_or(true);
            let hint = if bun_missing {
                if cfg!(target_os = "windows") {
                    "Install Bun first: powershell -c \"irm bun.sh/install.ps1 | iex\", then: cd apps/api && bun run codegen".to_string()
                } else {
                    "Install Bun first: curl -fsSL https://bun.sh/install | bash, then: cd apps/api && bun run codegen".to_string()
                }
            } else {
                "Run 'cd apps/api && bun run codegen' manually".to_string()
            };
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Warning,
                    code: "INIT_CODEGEN_FAILED".into(),
                    message: format!(
                        "Files created but manifest generation failed: {}",
                        diag.message
                    ),
                    span: None,
                    hint: Some(hint),
                }],
                json_mode,
            );
        }
    }

    let target_display = target.display().to_string();
    if json_mode {
        print_json(&InitOutput {
            code: "INIT_OK",
            name: app_name.clone(),
            path: target_display.clone(),
            frontend: format!("{:?}", frontend).to_lowercase(),
            files: written_files,
        });
    } else {
        print_next_steps(&target_display, frontend, deps_installed);
    }

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// Interactive frontend selection
// ---------------------------------------------------------------------------

fn prompt_frontend() -> Frontend {
    let options = [
        ("Next.js", Frontend::NextJs),
        ("React + Vite", Frontend::React),
        ("TanStack Start", Frontend::TanStack),
        ("No frontend (API only)", Frontend::None),
    ];

    println!();
    println!("Which frontend would you like?");
    for (i, (label, _)) in options.iter().enumerate() {
        let marker = if i == 0 { " (default)" } else { "" };
        println!("  {}) {}{}", i + 1, label, marker);
    }
    print!("Choose [1-{}, default 1]: ", options.len());
    let _ = std::io::stdout().flush();

    let mut input = String::new();
    let _ = std::io::stdin().read_line(&mut input);
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Frontend::NextJs;
    }
    if let Ok(n) = trimmed.parse::<usize>() {
        if n >= 1 && n <= options.len() {
            return options[n - 1].1;
        }
    }
    if let Some(f) = Frontend::parse(trimmed) {
        return f;
    }
    eprintln!("(unrecognized choice; defaulting to Next.js)");
    Frontend::NextJs
}

// ---------------------------------------------------------------------------
// shadcn CLI bridge
// ---------------------------------------------------------------------------

/// Run `bun install` at the workspace root. Returns true on success.
/// Required before `shadcn add` because shadcn's CLI inspects installed
/// peer deps to figure out which Tailwind / Radix versions to wire up.
fn run_bun_install(workspace_root: &Path, json_mode: bool) -> bool {
    let bun_available = std::process::Command::new("bun")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !bun_available {
        let install = if cfg!(target_os = "windows") {
            "powershell -c \"irm bun.sh/install.ps1 | iex\""
        } else {
            "curl -fsSL https://bun.sh/install | bash"
        };
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Warning,
                code: "INIT_NEEDS_BUN".into(),
                message: "Skipped `bun install` — `bun` not found".into(),
                span: None,
                hint: Some(format!(
                    "Install Bun ({install}), then run `bun install` in the project root."
                )),
            }],
            json_mode,
        );
        return false;
    }
    if !json_mode {
        println!("Installing dependencies (bun install)…");
    }
    let result = std::process::Command::new("bun")
        .current_dir(workspace_root)
        .arg("install")
        .arg("--silent")
        .status();
    match result {
        Ok(s) if s.success() => true,
        Ok(s) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Warning,
                    code: "INIT_BUN_INSTALL_FAILED".into(),
                    message: format!("`bun install` exited with status {s}"),
                    span: None,
                    hint: Some("Run `bun install` manually to see the full error.".into()),
                }],
                json_mode,
            );
            false
        }
        Err(e) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Warning,
                    code: "INIT_BUN_SPAWN_FAILED".into(),
                    message: format!("Could not run `bun install`: {e}"),
                    span: None,
                    hint: Some("Run `bun install` manually.".into()),
                }],
                json_mode,
            );
            false
        }
    }
}

/// Run `bunx shadcn@latest add <components> --yes` inside `web_dir`.
/// On failure, print a hint with the manual fallback command so users
/// know exactly what to type to recover. Returns true on success.
fn run_shadcn_add(web_dir: &Path, written: &mut Vec<String>, json_mode: bool) -> bool {
    if !json_mode {
        println!(
            "Installing shadcn components ({})…",
            SHADCN_COMPONENTS.join(", ")
        );
    }

    let mut cmd = std::process::Command::new("bunx");
    cmd.current_dir(web_dir)
        .arg("--bun")
        .arg("shadcn@latest")
        .arg("add")
        .args(SHADCN_COMPONENTS)
        .arg("--yes")
        .arg("--overwrite");

    let result = cmd.output();
    match result {
        Ok(out) if out.status.success() => {
            for c in SHADCN_COMPONENTS {
                written.push(format!("{}/components/ui/{c}.tsx", web_dir.display()));
            }
            true
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Warning,
                    code: "INIT_SHADCN_FAILED".into(),
                    message: format!(
                        "shadcn CLI exited with status {}: {}",
                        out.status,
                        stderr.lines().next().unwrap_or("(no output)")
                    ),
                    span: None,
                    hint: Some(format!(
                        "Re-run inside apps/web: bunx shadcn@latest add {}",
                        SHADCN_COMPONENTS.join(" ")
                    )),
                }],
                json_mode,
            );
            false
        }
        Err(e) => {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Warning,
                    code: "INIT_SHADCN_SPAWN_FAILED".into(),
                    message: format!("Could not run shadcn CLI: {e}"),
                    span: None,
                    hint: Some(format!(
                        "Re-run inside apps/web: bunx shadcn@latest add {}",
                        SHADCN_COMPONENTS.join(" ")
                    )),
                }],
                json_mode,
            );
            false
        }
    }
}

// ---------------------------------------------------------------------------
// File rendering
// ---------------------------------------------------------------------------

fn write_file(
    base: &Path,
    rel: &str,
    contents: &str,
    json_mode: bool,
    written: &mut Vec<String>,
) -> Result<(), ExitCode> {
    let path = base.join(rel);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Err(mkdir_err(parent, e, json_mode));
            }
        }
    }
    if let Err(e) = std::fs::write(&path, contents) {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "INIT_WRITE_FAILED".into(),
                message: format!("Could not write {}: {e}", path.display()),
                span: None,
                hint: None,
            }],
            json_mode,
        );
        return Err(ExitCode::Error);
    }
    written.push(path.display().to_string());
    Ok(())
}

fn write_frontend(
    frontend: Frontend,
    web_dir: &Path,
    app_name: &str,
    json_mode: bool,
    written: &mut Vec<String>,
) -> Result<(), ExitCode> {
    // Pin @pylonsync/* deps to the same version as this CLI binary so
    // the JS SDK's wire format always matches the running server. `*` and
    // `latest` both fail too easily when transitive peer deps in the
    // @pylonsync/next package don't line up with what npm has published.
    let pylon_version = env!("CARGO_PKG_VERSION");
    let render = |s: &str| {
        s.replace("__APP_NAME__", app_name)
            .replace("__PYLON_VERSION__", pylon_version)
    };
    let files: Vec<(&str, String)> = match frontend {
        Frontend::NextJs => vec![
            ("package.json", render(NEXTJS_PACKAGE_JSON)),
            ("tsconfig.json", NEXTJS_TSCONFIG.into()),
            ("next.config.js", NEXTJS_NEXT_CONFIG.into()),
            ("next-env.d.ts", NEXTJS_NEXT_ENV.into()),
            ("postcss.config.mjs", NEXTJS_POSTCSS.into()),
            ("components.json", NEXTJS_COMPONENTS_JSON.into()),
            ("lib/pylon.ts", render(NEXTJS_LIB_PYLON)),
            ("lib/utils.ts", SHARED_UTILS.into()),
            ("proxy.ts", render(NEXTJS_PROXY)),
            ("app/globals.css", NEXTJS_GLOBALS_CSS.into()),
            ("app/layout.tsx", render(NEXTJS_LAYOUT)),
            ("app/providers.tsx", NEXTJS_PROVIDERS.into()),
            ("app/page.tsx", render(NEXTJS_PAGE)),
            ("app/login/page.tsx", NEXTJS_LOGIN_PAGE.into()),
            ("app/login/form.tsx", NEXTJS_LOGIN_FORM.into()),
            ("app/login/actions.ts", render(NEXTJS_LOGIN_ACTIONS)),
            ("app/dashboard/layout.tsx", render(NEXTJS_DASHBOARD_LAYOUT)),
            ("app/dashboard/page.tsx", NEXTJS_DASHBOARD_PAGE.into()),
        ],
        Frontend::React => vec![
            ("package.json", render(REACT_PACKAGE_JSON)),
            ("tsconfig.json", REACT_TSCONFIG.into()),
            ("vite.config.ts", REACT_VITE_CONFIG.into()),
            ("index.html", render(REACT_INDEX_HTML)),
            ("src/main.tsx", REACT_MAIN.into()),
            ("src/App.tsx", REACT_APP.into()),
            ("src/Login.tsx", REACT_LOGIN.into()),
            ("src/Dashboard.tsx", render(REACT_DASHBOARD)),
            ("src/lib/pylon.ts", REACT_LIB_PYLON.into()),
        ],
        Frontend::TanStack => vec![
            ("package.json", render(TANSTACK_PACKAGE_JSON)),
            ("tsconfig.json", TANSTACK_TSCONFIG.into()),
            ("app.config.ts", TANSTACK_APP_CONFIG.into()),
            ("app/router.tsx", TANSTACK_ROUTER.into()),
            ("app/client.tsx", TANSTACK_CLIENT.into()),
            ("app/ssr.tsx", TANSTACK_SSR.into()),
            ("app/routes/__root.tsx", render(TANSTACK_ROOT_ROUTE)),
            ("app/routes/index.tsx", render(TANSTACK_INDEX_ROUTE)),
        ],
        Frontend::None => vec![],
    };
    for (name, contents) in files {
        write_file(web_dir, name, &contents, json_mode, written)?;
    }
    Ok(())
}

fn mkdir_err(path: &Path, e: std::io::Error, json_mode: bool) -> ExitCode {
    print_diagnostics(
        &[Diagnostic {
            severity: Severity::Error,
            code: "INIT_MKDIR_FAILED".into(),
            message: format!("Could not create directory \"{}\": {e}", path.display()),
            span: None,
            hint: None,
        }],
        json_mode,
    );
    ExitCode::Error
}

fn rel_path(base: &Path, full: &Path) -> String {
    full.strip_prefix(base)
        .map(|p| format!("{}/{}", base.display(), p.display()))
        .unwrap_or_else(|_| full.display().to_string())
}

// ---------------------------------------------------------------------------
// Generated workspace root files
// ---------------------------------------------------------------------------

fn workspace_root_package_json(app_name: &str, frontend: Frontend) -> String {
    let scripts = match frontend {
        Frontend::None => serde_json::json!({
            "dev": "bun --filter api dev",
            "codegen": "bun --filter api codegen",
        }),
        _ => serde_json::json!({
            "dev": "bun run --parallel dev:api dev:web",
            "dev:api": "bun --filter api dev",
            "dev:web": "bun --filter web dev",
            "build": "bun --filter web build",
            "codegen": "bun --filter api codegen",
        }),
    };
    serde_json::to_string_pretty(&serde_json::json!({
        "name": app_name,
        "version": "0.1.0",
        "private": true,
        "type": "module",
        "workspaces": ["apps/*"],
        "scripts": scripts,
    }))
    .unwrap()
        + "\n"
}

fn api_package_json(_app_name: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "name": "api",
        "version": "0.1.0",
        "private": true,
        "type": "module",
        "scripts": {
            "dev": "pylon dev app.ts",
            "codegen": "pylon codegen app.ts --out pylon.manifest.json",
            "doctor": "pylon doctor pylon.manifest.json",
            "check": "tsc -p tsconfig.json --noEmit"
        }
    }))
    .unwrap()
        + "\n"
}

const WORKSPACE_GITIGNORE: &str = "node_modules/\n\
                                   .pylon/\n\
                                   pylon.dev.db*\n\
                                   pylon.sessions.db*\n\
                                   pylon.jobs.db*\n\
                                   .next/\n\
                                   .vinxi/\n\
                                   .output/\n\
                                   dist/\n\
                                   .DS_Store\n\
                                   *.log\n\
                                   .env\n\
                                   .env.local\n";

fn workspace_readme(app_name: &str, frontend: Frontend) -> String {
    let frontend_section = match frontend {
        Frontend::NextJs => "\n## Web — Next.js\n\nLocated at `apps/web/`. Edit `app/page.tsx` to change the home page. The dev server proxies `/api/*` to the Pylon backend on `:4321` (see `next.config.js`).\n",
        Frontend::React => "\n## Web — React + Vite\n\nLocated at `apps/web/`. Edit `src/App.tsx` to change the home page. The Vite dev server proxies `/api/*` to the Pylon backend on `:4321` (see `vite.config.ts`).\n",
        Frontend::TanStack => "\n## Web — TanStack Start\n\nLocated at `apps/web/`. Edit `app/routes/index.tsx` to change the home page. Vinxi proxies `/api/*` to the Pylon backend on `:4321` (see `app.config.ts`).\n",
        Frontend::None => "",
    };
    let frontend_label = frontend.label();
    format!(
        "# {app_name}\n\nA Pylon monorepo: Pylon backend in `apps/api`, {frontend_label} frontend in `apps/web`.\n\n## Run\n\n```bash\nbun install\nbun dev          # starts both api + web\n```\n\nOpen http://localhost:3000 (frontend) and http://localhost:4321/studio (Pylon Studio).\n\n## Layout\n\n```\n{app_name}/\n├── apps/\n│   ├── api/      Pylon backend (schema, policies, functions)\n│   └── web/      {frontend_label} frontend\n├── package.json  Workspace root with scripts\n└── README.md\n```\n\n## API\n\nLocated at `apps/api/`. The schema lives in `app.ts`; server functions go in `functions/*.ts`. Generated `pylon.manifest.json` and `pylon.client.ts` get regenerated on every `pylon dev` reload.\n\nSee https://docs.pylonsync.com for the full reference.\n{frontend_section}\n## Deploy\n\nThe API and the frontend deploy independently. See https://docs.pylonsync.com/operations/deploy for the API and your frontend's docs (Next.js / Vite / TanStack Start) for the web app.\n"
    )
}

// ---------------------------------------------------------------------------
// Next-steps printer
// ---------------------------------------------------------------------------

fn print_next_steps(target_display: &str, frontend: Frontend, deps_installed: bool) {
    println!();
    println!("✓ Created {target_display}/");
    println!("    apps/api/        Pylon backend");
    if frontend != Frontend::None {
        println!("    apps/web/        {} frontend", frontend.label());
    }
    println!();
    println!("Next steps:");
    println!("  cd {target_display}");
    if !deps_installed {
        println!("  bun install");
    }
    if frontend != Frontend::None {
        println!("  bun dev          # starts both api (:4321) + web (:3000)");
    } else {
        println!("  bun dev          # starts pylon api on :4321");
    }
    println!();
    println!("Inspector: http://localhost:4321/studio");
    if frontend != Frontend::None {
        println!("Frontend:  http://localhost:3000");
    }
}
