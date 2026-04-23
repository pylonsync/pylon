use pylon_kernel::ExitCode;

use crate::manifest::load_manifest;
use crate::output::{print_diagnostics, print_json};

pub fn run(args: &[String], json_mode: bool) -> ExitCode {
    let path = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "explain")
        .next()
        .map(|s| s.as_str());

    let manifest_path = path.unwrap_or("pylon.manifest.json");

    let manifest = match load_manifest(manifest_path) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    if json_mode {
        print_json(&manifest);
    } else {
        println!(
            "App: {} v{} (manifest v{})",
            manifest.name, manifest.version, manifest.manifest_version
        );
        println!();
        println!("Entities:");
        for entity in &manifest.entities {
            println!("  {}", entity.name);
            for field in &entity.fields {
                let mut modifiers = Vec::new();
                if field.optional {
                    modifiers.push("optional");
                }
                if field.unique {
                    modifiers.push("unique");
                }
                let mod_str = if modifiers.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", modifiers.join(", "))
                };
                println!("    {}: {}{}", field.name, field.field_type, mod_str);
            }
            for index in &entity.indexes {
                let unique_str = if index.unique { " [unique]" } else { "" };
                println!(
                    "    index {}: [{}]{}",
                    index.name,
                    index.fields.join(", "),
                    unique_str
                );
            }
        }
        if !manifest.queries.is_empty() {
            println!();
            println!("Queries:");
            for query in &manifest.queries {
                if query.input.is_empty() {
                    println!("  {}", query.name);
                } else {
                    let inputs: Vec<String> = query
                        .input
                        .iter()
                        .map(|f| format!("{}: {}", f.name, f.field_type))
                        .collect();
                    println!("  {}({})", query.name, inputs.join(", "));
                }
            }
        }

        if !manifest.actions.is_empty() {
            println!();
            println!("Actions:");
            for action in &manifest.actions {
                if action.input.is_empty() {
                    println!("  {}", action.name);
                } else {
                    let inputs: Vec<String> = action
                        .input
                        .iter()
                        .map(|f| format!("{}: {}", f.name, f.field_type))
                        .collect();
                    println!("  {}({})", action.name, inputs.join(", "));
                }
            }
        }

        if !manifest.policies.is_empty() {
            println!();
            println!("Policies:");
            for policy in &manifest.policies {
                let target = match (&policy.entity, &policy.action) {
                    (Some(e), Some(a)) => format!("entity={e}, action={a}"),
                    (Some(e), None) => format!("entity={e}"),
                    (None, Some(a)) => format!("action={a}"),
                    (None, None) => "no target".into(),
                };
                println!("  {} [{}] -> {}", policy.name, target, policy.allow);
            }
        }

        println!();
        println!("Routes:");
        for route in &manifest.routes {
            let mut parts = vec![route.mode.clone()];
            if let Some(ref q) = route.query {
                parts.push(format!("query={q}"));
            }
            if let Some(ref a) = route.auth {
                parts.push(format!("auth={a}"));
            }
            println!("  {} ({})", route.path, parts.join(", "));
        }
    }

    ExitCode::Ok
}
