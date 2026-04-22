use statecraft_core::{Diagnostic, ExitCode, Severity};

use crate::manifest::{load_manifest, validate_all};
use crate::output::{print_diagnostics, print_json};

// ---------------------------------------------------------------------------
// Deployment target
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeployTarget {
    /// Default: just the manifest + client bindings + static pages.
    Default,
    /// Generate a Dockerfile.
    Docker,
    /// Generate a Dockerfile + fly.toml.
    Fly,
    /// Generate a docker-compose.yml + Dockerfile.
    Compose,
    /// Generate a Cloudflare Workers wrangler.toml. Experimental — see
    /// crates/workers/README.md for what works.
    Workers,
    /// Generate a systemd unit file for VPS deploys.
    Systemd,
}

impl DeployTarget {
    fn from_arg(s: &str) -> Option<Self> {
        match s {
            "docker" => Some(Self::Docker),
            "fly" => Some(Self::Fly),
            "compose" => Some(Self::Compose),
            "workers" => Some(Self::Workers),
            "systemd" => Some(Self::Systemd),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// File generators
// ---------------------------------------------------------------------------

fn generate_dockerfile() -> String {
    r#"FROM rust:1.82-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/statecraft /usr/local/bin/
COPY --from=builder /app/statecraft.manifest.json /app/
EXPOSE 4321
CMD ["statecraft", "dev", "--once"]
"#
    .to_string()
}

fn generate_fly_toml(app_name: &str) -> String {
    format!(
        r#"app = "{app_name}"
primary_region = "iad"

[http_service]
  internal_port = 4321
  force_https = true

[build]
  dockerfile = "Dockerfile"
"#
    )
}

fn generate_wrangler_toml(app_name: &str) -> String {
    format!(
        r#"name = "{app_name}"
main = "build/worker.js"
compatibility_date = "2025-01-01"

# Created database with: wrangler d1 create {app_name}-db
# Then paste the database_id printed by wrangler below.
[[d1_databases]]
binding = "STATECRAFT_DB"
database_name = "{app_name}-db"
database_id = "REPLACE_WITH_D1_DATABASE_ID"

# Optional: persistent file storage via R2
# [[r2_buckets]]
# binding = "STATECRAFT_FILES"
# bucket_name = "{app_name}-files"

# Per-room WebSocket Durable Object (experimental)
# [[durable_objects.bindings]]
# name = "ROOM"
# class_name = "RoomDO"
"#
    )
}

fn generate_systemd_unit(app_name: &str) -> String {
    format!(
        r#"[Unit]
Description=statecraft ({app_name})
After=network.target

[Service]
Type=simple
User=statecraft
Group=statecraft
WorkingDirectory=/var/lib/statecraft
ExecStart=/usr/local/bin/statecraft dev
Restart=on-failure
RestartSec=5

# Hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/statecraft

# Environment
Environment=STATECRAFT_PORT=4321
Environment=STATECRAFT_DB_PATH=/var/lib/statecraft/statecraft.db
Environment=STATECRAFT_FILES_DIR=/var/lib/statecraft/uploads
Environment=STATECRAFT_SESSION_DB=/var/lib/statecraft/sessions.db
Environment=STATECRAFT_DEV_MODE=false
# EnvironmentFile=/etc/statecraft/secrets  # STATECRAFT_ADMIN_TOKEN, OAuth keys, etc.

[Install]
WantedBy=multi-user.target
"#
    )
}

fn generate_docker_compose() -> String {
    r#"services:
  app:
    build: .
    ports:
      - "4321:4321"
    environment:
      - DATABASE_URL=postgres://statecraft:statecraft@db:5432/statecraft
      - STATECRAFT_ADMIN_TOKEN=${STATECRAFT_ADMIN_TOKEN}
    depends_on:
      db:
        condition: service_started

  db:
    image: postgres:16
    environment:
      - POSTGRES_USER=statecraft
      - POSTGRES_PASSWORD=statecraft
      - POSTGRES_DB=statecraft
    volumes:
      - pgdata:/var/lib/postgresql/data

volumes:
  pgdata:
"#
    .to_string()
}

// ---------------------------------------------------------------------------
// Command entry point
// ---------------------------------------------------------------------------

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "deploy")
        .map(|s| s.as_str())
        .collect();

    let manifest_path = positional.first().copied().unwrap_or("statecraft.manifest.json");

    let out_dir = args
        .windows(2)
        .find(|w| w[0] == "--out")
        .map(|w| w[1].as_str())
        .unwrap_or("deploy");

    let target = args
        .windows(2)
        .find(|w| w[0] == "--target")
        .map(|w| w[1].as_str())
        .and_then(|s| {
            let t = DeployTarget::from_arg(s);
            if t.is_none() {
                // Will be reported as a diagnostic below.
            }
            t
        });

    // Validate --target value if the flag was provided but unrecognised.
    if let Some(raw) = args.windows(2).find(|w| w[0] == "--target").map(|w| w[1].as_str()) {
        if DeployTarget::from_arg(raw).is_none() {
            print_diagnostics(
                &[Diagnostic {
                    severity: Severity::Error,
                    code: "INVALID_TARGET".into(),
                    message: format!(
                        "Unknown deploy target \"{raw}\". Valid targets: docker, fly, compose, workers, systemd"
                    ),
                    span: None,
                    hint: Some(
                        "Use --target docker | fly | compose | workers | systemd".into(),
                    ),
                }],
                json_mode,
            );
            return ExitCode::Error;
        }
    }

    let target = target.unwrap_or(DeployTarget::Default);

    let manifest = match load_manifest(manifest_path) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    let diagnostics = validate_all(&manifest);
    let has_errors = diagnostics.iter().any(|d| d.severity == Severity::Error);

    if has_errors {
        print_diagnostics(&diagnostics, json_mode);
        return ExitCode::Error;
    }

    // Create deploy directory.
    let out_path = std::path::Path::new(out_dir);
    if let Err(e) = std::fs::create_dir_all(out_path) {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "DEPLOY_DIR_FAILED".into(),
                message: format!("Could not create deploy directory: {e}"),
                span: None,
                hint: None,
            }],
            json_mode,
        );
        return ExitCode::Error;
    }

    // Write manifest to deploy dir.
    let manifest_out = out_path.join("statecraft.manifest.json");
    let manifest_json = serde_json::to_string_pretty(&manifest).unwrap_or_default();
    if let Err(e) = std::fs::write(&manifest_out, format!("{manifest_json}\n")) {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Error,
                code: "DEPLOY_WRITE_FAILED".into(),
                message: format!("Could not write manifest: {e}"),
                span: None,
                hint: None,
            }],
            json_mode,
        );
        return ExitCode::Error;
    }

    // Write client bindings.
    let client_ts = crate::client_codegen::generate_client_ts(&manifest);
    let _ = std::fs::write(out_path.join("statecraft.client.ts"), &client_ts);

    // Generate static pages if any.
    let static_pages = statecraft_staticgen::generate_static_pages(&manifest);
    if !static_pages.is_empty() {
        let static_dir = out_path.join("static");
        let _ = statecraft_staticgen::write_pages(&static_pages, &static_dir);
    }

    // Write a small deploy info file.
    let deploy_info = serde_json::json!({
        "name": manifest.name,
        "version": manifest.version,
        "manifest_version": manifest.manifest_version,
        "entities": manifest.entities.len(),
        "queries": manifest.queries.len(),
        "actions": manifest.actions.len(),
        "policies": manifest.policies.len(),
        "routes": manifest.routes.len(),
        "static_pages": static_pages.len(),
    });
    let _ = std::fs::write(
        out_path.join("deploy.json"),
        serde_json::to_string_pretty(&deploy_info).unwrap_or_default(),
    );

    // -----------------------------------------------------------------------
    // Target-specific file generation
    // -----------------------------------------------------------------------

    let mut generated_files: Vec<String> = vec![
        "statecraft.manifest.json".into(),
        "statecraft.client.ts".into(),
        "deploy.json".into(),
    ];

    match target {
        DeployTarget::Docker => {
            let dockerfile = generate_dockerfile();
            write_or_fail(out_path, "Dockerfile", &dockerfile, json_mode);
            generated_files.push("Dockerfile".into());
        }
        DeployTarget::Fly => {
            let dockerfile = generate_dockerfile();
            write_or_fail(out_path, "Dockerfile", &dockerfile, json_mode);
            generated_files.push("Dockerfile".into());

            let app_name = sanitize_app_name(&manifest.name);
            let fly_toml = generate_fly_toml(&app_name);
            write_or_fail(out_path, "fly.toml", &fly_toml, json_mode);
            generated_files.push("fly.toml".into());
        }
        DeployTarget::Compose => {
            let dockerfile = generate_dockerfile();
            write_or_fail(out_path, "Dockerfile", &dockerfile, json_mode);
            generated_files.push("Dockerfile".into());

            let compose = generate_docker_compose();
            write_or_fail(out_path, "docker-compose.yml", &compose, json_mode);
            generated_files.push("docker-compose.yml".into());
        }
        DeployTarget::Workers => {
            let app_name = sanitize_app_name(&manifest.name);
            let wrangler = generate_wrangler_toml(&app_name);
            write_or_fail(out_path, "wrangler.toml", &wrangler, json_mode);
            generated_files.push("wrangler.toml".into());
        }
        DeployTarget::Systemd => {
            let app_name = sanitize_app_name(&manifest.name);
            let unit = generate_systemd_unit(&app_name);
            write_or_fail(out_path, "statecraft.service", &unit, json_mode);
            generated_files.push("statecraft.service".into());
        }
        DeployTarget::Default => {}
    }

    // -----------------------------------------------------------------------
    // Output
    // -----------------------------------------------------------------------

    if json_mode {
        print_json(&serde_json::json!({
            "code": "DEPLOY_OK",
            "out_dir": out_dir,
            "manifest": manifest_path,
            "target": format!("{target:?}"),
            "static_pages": static_pages.len(),
            "files": generated_files,
        }));
    } else {
        println!("Deploy package created: {out_dir}/");
        for f in &generated_files {
            println!("  {f}");
        }
        if !static_pages.is_empty() {
            println!("  static/ ({} pages)", static_pages.len());
        }
        println!();

        match target {
            DeployTarget::Docker => {
                println!("To build and run:");
                println!("  docker build -t statecraft-app {out_dir}/");
                println!("  docker run -p 4321:4321 statecraft-app");
            }
            DeployTarget::Fly => {
                println!("To deploy to Fly.io:");
                println!("  cd {out_dir} && fly launch");
            }
            DeployTarget::Compose => {
                println!("To run with Docker Compose:");
                println!("  cd {out_dir} && docker compose up");
            }
            DeployTarget::Workers => {
                println!("To deploy to Cloudflare Workers (experimental):");
                println!("  cd {out_dir}");
                println!("  wrangler d1 create $(basename $(pwd))-db");
                println!("  # paste database_id into wrangler.toml");
                println!("  wrangler deploy");
                println!();
                println!("See crates/workers/README.md for current limitations.");
            }
            DeployTarget::Systemd => {
                println!("To install on a Linux VPS:");
                println!("  sudo cp {out_dir}/statecraft.service /etc/systemd/system/");
                println!("  sudo useradd --system --home /var/lib/statecraft statecraft");
                println!("  sudo mkdir -p /var/lib/statecraft && sudo chown statecraft: /var/lib/statecraft");
                println!("  sudo systemctl enable --now statecraft");
            }
            DeployTarget::Default => {
                println!("To run the server:");
                println!("  statecraft dev {manifest_path}");
                println!();
                println!("For containerized deployment, use --target:");
                println!("  statecraft deploy --target docker     # Dockerfile");
                println!("  statecraft deploy --target fly        # Dockerfile + fly.toml");
                println!("  statecraft deploy --target compose    # docker-compose.yml + Dockerfile");
                println!("  statecraft deploy --target workers    # Cloudflare wrangler.toml (experimental)");
                println!("  statecraft deploy --target systemd    # systemd unit for VPS install");
            }
        }
    }

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write a file into the output directory, printing a diagnostic on failure.
fn write_or_fail(
    out_path: &std::path::Path,
    filename: &str,
    content: &str,
    json_mode: bool,
) {
    let path = out_path.join(filename);
    if let Err(e) = std::fs::write(&path, content) {
        print_diagnostics(
            &[Diagnostic {
                severity: Severity::Warning,
                code: "DEPLOY_WRITE_FAILED".into(),
                message: format!("Could not write {filename}: {e}"),
                span: None,
                hint: None,
            }],
            json_mode,
        );
    }
}

/// Sanitize a manifest name into a valid Fly.io app name.
///
/// Fly app names must be lowercase alphanumeric with hyphens, no underscores
/// or spaces.
fn sanitize_app_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dockerfile_contains_expected_stages() {
        let df = generate_dockerfile();
        assert!(df.contains("FROM rust:1.82-slim AS builder"));
        assert!(df.contains("FROM debian:bookworm-slim"));
        assert!(df.contains("EXPOSE 4321"));
        assert!(df.contains("cargo build --release"));
        assert!(df.contains("statecraft.manifest.json"));
    }

    #[test]
    fn fly_toml_contains_app_name_and_port() {
        let toml = generate_fly_toml("my-app");
        assert!(toml.contains("app = \"my-app\""));
        assert!(toml.contains("internal_port = 4321"));
        assert!(toml.contains("force_https = true"));
        assert!(toml.contains("primary_region = \"iad\""));
    }

    #[test]
    fn docker_compose_contains_services() {
        let dc = generate_docker_compose();
        assert!(dc.contains("services:"));
        assert!(dc.contains("postgres:16"));
        assert!(dc.contains("DATABASE_URL=postgres://statecraft:statecraft@db:5432/statecraft"));
        assert!(dc.contains("STATECRAFT_ADMIN_TOKEN"));
        assert!(dc.contains("pgdata:"));
    }

    #[test]
    fn sanitize_app_name_handles_spaces_and_underscores() {
        assert_eq!(sanitize_app_name("My App"), "my-app");
        assert_eq!(sanitize_app_name("my_app"), "my-app");
        assert_eq!(sanitize_app_name("already-good"), "already-good");
        assert_eq!(sanitize_app_name("UPPER"), "upper");
    }

    #[test]
    fn deploy_target_parsing() {
        assert_eq!(DeployTarget::from_arg("docker"), Some(DeployTarget::Docker));
        assert_eq!(DeployTarget::from_arg("fly"), Some(DeployTarget::Fly));
        assert_eq!(DeployTarget::from_arg("compose"), Some(DeployTarget::Compose));
        assert_eq!(DeployTarget::from_arg("workers"), Some(DeployTarget::Workers));
        assert_eq!(DeployTarget::from_arg("systemd"), Some(DeployTarget::Systemd));
        assert_eq!(DeployTarget::from_arg("unknown"), None);
    }

    #[test]
    fn wrangler_toml_contains_required_fields() {
        let w = generate_wrangler_toml("my-app");
        assert!(w.contains("name = \"my-app\""));
        assert!(w.contains("d1_databases"));
        assert!(w.contains("STATECRAFT_DB"));
        assert!(w.contains("compatibility_date"));
    }

    #[test]
    fn systemd_unit_contains_hardening() {
        let u = generate_systemd_unit("statecraft-prod");
        assert!(u.contains("ExecStart=/usr/local/bin/statecraft"));
        assert!(u.contains("NoNewPrivileges=true"));
        assert!(u.contains("ProtectSystem=strict"));
        assert!(u.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn dockerfile_has_ca_certificates() {
        let df = generate_dockerfile();
        assert!(df.contains("ca-certificates"));
    }

    #[test]
    fn docker_compose_has_depends_on() {
        let dc = generate_docker_compose();
        assert!(dc.contains("depends_on:"));
    }
}
