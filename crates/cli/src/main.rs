use agentdb_core::{
    AppManifest, Diagnostic, ExitCode, ManifestEntity, ManifestField, ManifestIndex, ManifestRoute,
    Severity, VERSION,
};
use agentdb_schema::{Entity, Field, FieldType, Index, Schema};

fn main() {
    std::process::exit(run().as_i32());
}

fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json_mode = args.iter().any(|a| a == "--json");
    let command = args.iter().find(|a| !a.starts_with('-')).map(|s| s.as_str());

    match command {
        Some("doctor") => cmd_doctor(&args, json_mode),
        Some("explain") => cmd_explain(&args, json_mode),
        Some("version") => cmd_version(json_mode),
        Some(cmd) => {
            eprintln!("unknown command: {cmd}");
            print_usage();
            ExitCode::Usage
        }
        None => {
            if args.iter().any(|a| a == "--version") {
                cmd_version(json_mode)
            } else {
                print_usage();
                ExitCode::Ok
            }
        }
    }
}

// ---------------------------------------------------------------------------
// doctor — validate a manifest file
// ---------------------------------------------------------------------------

fn cmd_doctor(args: &[String], json_mode: bool) -> ExitCode {
    let path = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "doctor")
        .next()
        .map(|s| s.as_str());

    let manifest_path = path.unwrap_or("agentdb.manifest.json");

    let contents = match std::fs::read_to_string(manifest_path) {
        Ok(c) => c,
        Err(e) => {
            let diag = Diagnostic {
                severity: Severity::Error,
                code: "MANIFEST_READ_FAILED".into(),
                message: format!("Could not read manifest: {manifest_path}: {e}"),
                span: None,
                hint: Some("Provide a valid manifest path or run from the project root".into()),
            };
            print_diagnostics(&[diag], json_mode);
            return ExitCode::Error;
        }
    };

    let manifest = match parse_manifest(&contents, manifest_path) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    let schema = manifest_to_schema(&manifest);
    let mut diagnostics = agentdb_schema::validate(&schema);

    if diagnostics.is_empty() {
        diagnostics.push(Diagnostic {
            severity: Severity::Info,
            code: "DOCTOR_OK".into(),
            message: format!(
                "Manifest OK: {} entities, {} routes",
                manifest.entities.len(),
                manifest.routes.len()
            ),
            span: None,
            hint: None,
        });
    }

    let has_errors = diagnostics.iter().any(|d| d.severity == Severity::Error);
    print_diagnostics(&diagnostics, json_mode);
    if has_errors { ExitCode::Error } else { ExitCode::Ok }
}

// ---------------------------------------------------------------------------
// explain — print a structured summary of a manifest
// ---------------------------------------------------------------------------

fn cmd_explain(args: &[String], json_mode: bool) -> ExitCode {
    let path = args
        .iter()
        .filter(|a| !a.starts_with('-') && *a != "explain")
        .next()
        .map(|s| s.as_str());

    let manifest_path = path.unwrap_or("agentdb.manifest.json");

    let contents = match std::fs::read_to_string(manifest_path) {
        Ok(c) => c,
        Err(e) => {
            let diag = Diagnostic {
                severity: Severity::Error,
                code: "MANIFEST_READ_FAILED".into(),
                message: format!("Could not read manifest: {manifest_path}: {e}"),
                span: None,
                hint: Some("Provide a valid manifest path or run from the project root".into()),
            };
            print_diagnostics(&[diag], json_mode);
            return ExitCode::Error;
        }
    };

    let manifest = match parse_manifest(&contents, manifest_path) {
        Ok(m) => m,
        Err(diags) => {
            print_diagnostics(&diags, json_mode);
            return ExitCode::Error;
        }
    };

    if json_mode {
        // Re-serialize as structured JSON output
        println!("{contents}");
    } else {
        println!("App: {} v{}", manifest.name, manifest.version);
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
                println!("    index {}: [{}]{}", index.name, index.fields.join(", "), unique_str);
            }
        }
        println!();
        println!("Routes:");
        for route in &manifest.routes {
            println!("  {} ({})", route.path, route.mode);
        }
    }

    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// version
// ---------------------------------------------------------------------------

fn cmd_version(json_mode: bool) -> ExitCode {
    if json_mode {
        println!("{{\"version\":\"{VERSION}\"}}");
    } else {
        println!("agentdb {VERSION}");
    }
    ExitCode::Ok
}

// ---------------------------------------------------------------------------
// Minimal JSON manifest parser (no serde dependency)
// ---------------------------------------------------------------------------

fn parse_manifest(contents: &str, path: &str) -> Result<AppManifest, Vec<Diagnostic>> {
    // Minimal hand-rolled JSON parser for the canonical manifest shape.
    // This avoids pulling in serde/serde_json for the first slice.
    // It's intentionally limited to the exact shape we produce.

    let trimmed = contents.trim();
    let err = |msg: String| -> Vec<Diagnostic> {
        vec![Diagnostic {
            severity: Severity::Error,
            code: "MANIFEST_PARSE_ERROR".into(),
            message: msg,
            span: Some(agentdb_core::Span {
                file: path.into(),
                line: None,
                column: None,
            }),
            hint: Some("Ensure the manifest is valid JSON matching the canonical schema".into()),
        }]
    };

    // We'll use a very simple approach: find known keys and extract values.
    // This is NOT a general JSON parser — it only handles our canonical shape.
    let get_string = |key: &str| -> Option<String> {
        let pattern = format!("\"{}\"", key);
        let idx = trimmed.find(&pattern)?;
        let after_key = &trimmed[idx + pattern.len()..];
        let colon = after_key.find(':')?;
        let after_colon = after_key[colon + 1..].trim_start();
        if !after_colon.starts_with('"') {
            return None;
        }
        let start = 1;
        let end = after_colon[start..].find('"')?;
        Some(after_colon[start..start + end].to_string())
    };

    let name = get_string("name").ok_or_else(|| err("Missing \"name\" field".into()))?;
    let version = get_string("version").ok_or_else(|| err("Missing \"version\" field".into()))?;

    // Parse entities array
    let entities = parse_entities_array(trimmed).map_err(|msg| err(msg))?;

    // Parse routes array
    let routes = parse_routes_array(trimmed).map_err(|msg| err(msg))?;

    Ok(AppManifest {
        name,
        version,
        entities,
        routes,
    })
}

fn parse_entities_array(json: &str) -> Result<Vec<ManifestEntity>, String> {
    let key = "\"entities\"";
    let idx = match json.find(key) {
        Some(i) => i,
        None => return Ok(vec![]),
    };
    let after = &json[idx + key.len()..];
    let arr_start = after.find('[').ok_or("Expected '[' after entities key")?;
    let arr_content = &after[arr_start..];
    let arr_end = find_matching_bracket(arr_content).ok_or("Unmatched '[' in entities")?;
    let arr_inner = &arr_content[1..arr_end];

    let mut entities = Vec::new();
    let mut pos = 0;
    while pos < arr_inner.len() {
        let obj_start = match arr_inner[pos..].find('{') {
            Some(i) => pos + i,
            None => break,
        };
        let obj_end = obj_start
            + find_matching_brace(&arr_inner[obj_start..])
                .ok_or("Unmatched '{' in entity object")?;
        let obj = &arr_inner[obj_start..=obj_end];
        entities.push(parse_entity_object(obj)?);
        pos = obj_end + 1;
    }

    Ok(entities)
}

fn parse_entity_object(obj: &str) -> Result<ManifestEntity, String> {
    let name = extract_string_value(obj, "name").unwrap_or_default();
    let fields = parse_fields_array(obj)?;
    let indexes = parse_indexes_array(obj)?;
    Ok(ManifestEntity {
        name,
        fields,
        indexes,
    })
}

fn parse_fields_array(obj: &str) -> Result<Vec<ManifestField>, String> {
    let key = "\"fields\"";
    let idx = match obj.find(key) {
        Some(i) => i,
        None => return Ok(vec![]),
    };
    let after = &obj[idx + key.len()..];
    let arr_start = after.find('[').ok_or("Expected '[' after fields key")?;
    let arr_content = &after[arr_start..];
    let arr_end = find_matching_bracket(arr_content).ok_or("Unmatched '[' in fields")?;
    let arr_inner = &arr_content[1..arr_end];

    let mut fields = Vec::new();
    let mut pos = 0;
    while pos < arr_inner.len() {
        let obj_start = match arr_inner[pos..].find('{') {
            Some(i) => pos + i,
            None => break,
        };
        let obj_end = obj_start
            + find_matching_brace(&arr_inner[obj_start..])
                .ok_or("Unmatched '{' in field object")?;
        let fobj = &arr_inner[obj_start..=obj_end];
        let name = extract_string_value(fobj, "name").unwrap_or_default();
        let field_type = extract_string_value(fobj, "type").unwrap_or_default();
        let optional = extract_bool_value(fobj, "optional").unwrap_or(false);
        let unique = extract_bool_value(fobj, "unique").unwrap_or(false);
        fields.push(ManifestField {
            name,
            field_type,
            optional,
            unique,
        });
        pos = obj_end + 1;
    }

    Ok(fields)
}

fn parse_indexes_array(obj: &str) -> Result<Vec<ManifestIndex>, String> {
    let key = "\"indexes\"";
    let idx = match obj.find(key) {
        Some(i) => i,
        None => return Ok(vec![]),
    };
    let after = &obj[idx + key.len()..];
    let arr_start = after.find('[').ok_or("Expected '[' after indexes key")?;
    let arr_content = &after[arr_start..];
    let arr_end = find_matching_bracket(arr_content).ok_or("Unmatched '[' in indexes")?;
    let arr_inner = &arr_content[1..arr_end];

    let mut indexes = Vec::new();
    let mut pos = 0;
    while pos < arr_inner.len() {
        let obj_start = match arr_inner[pos..].find('{') {
            Some(i) => pos + i,
            None => break,
        };
        let obj_end = obj_start
            + find_matching_brace(&arr_inner[obj_start..])
                .ok_or("Unmatched '{' in index object")?;
        let iobj = &arr_inner[obj_start..=obj_end];
        let name = extract_string_value(iobj, "name").unwrap_or_default();
        let unique = extract_bool_value(iobj, "unique").unwrap_or(false);
        let fields = extract_string_array(iobj, "fields").unwrap_or_default();
        indexes.push(ManifestIndex {
            name,
            fields,
            unique,
        });
        pos = obj_end + 1;
    }

    Ok(indexes)
}

fn parse_routes_array(json: &str) -> Result<Vec<ManifestRoute>, String> {
    let key = "\"routes\"";
    let idx = match json.find(key) {
        Some(i) => i,
        None => return Ok(vec![]),
    };
    let after = &json[idx + key.len()..];
    let arr_start = after.find('[').ok_or("Expected '[' after routes key")?;
    let arr_content = &after[arr_start..];
    let arr_end = find_matching_bracket(arr_content).ok_or("Unmatched '[' in routes")?;
    let arr_inner = &arr_content[1..arr_end];

    let mut routes = Vec::new();
    let mut pos = 0;
    while pos < arr_inner.len() {
        let obj_start = match arr_inner[pos..].find('{') {
            Some(i) => pos + i,
            None => break,
        };
        let obj_end = obj_start
            + find_matching_brace(&arr_inner[obj_start..])
                .ok_or("Unmatched '{' in route object")?;
        let robj = &arr_inner[obj_start..=obj_end];
        let path = extract_string_value(robj, "path").unwrap_or_default();
        let mode = extract_string_value(robj, "mode").unwrap_or_default();
        routes.push(ManifestRoute { path, mode });
        pos = obj_end + 1;
    }

    Ok(routes)
}

// ---------------------------------------------------------------------------
// JSON helper functions
// ---------------------------------------------------------------------------

fn find_matching_bracket(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_string_value(obj: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\"", key);
    let idx = obj.find(&pattern)?;
    let after = &obj[idx + pattern.len()..];
    let colon = after.find(':')?;
    let after_colon = after[colon + 1..].trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let end = after_colon[1..].find('"')?;
    Some(after_colon[1..1 + end].to_string())
}

fn extract_bool_value(obj: &str, key: &str) -> Option<bool> {
    let pattern = format!("\"{}\"", key);
    let idx = obj.find(&pattern)?;
    let after = &obj[idx + pattern.len()..];
    let colon = after.find(':')?;
    let after_colon = after[colon + 1..].trim_start();
    if after_colon.starts_with("true") {
        Some(true)
    } else if after_colon.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn extract_string_array(obj: &str, key: &str) -> Option<Vec<String>> {
    let pattern = format!("\"{}\"", key);
    let idx = obj.find(&pattern)?;
    let after = &obj[idx + pattern.len()..];
    let arr_start = after.find('[')?;
    let arr_content = &after[arr_start..];
    let arr_end = find_matching_bracket(arr_content)?;
    let inner = &arr_content[1..arr_end];

    let mut result = Vec::new();
    let mut pos = 0;
    while pos < inner.len() {
        let quote_start = match inner[pos..].find('"') {
            Some(i) => pos + i,
            None => break,
        };
        let quote_end = match inner[quote_start + 1..].find('"') {
            Some(i) => quote_start + 1 + i,
            None => break,
        };
        result.push(inner[quote_start + 1..quote_end].to_string());
        pos = quote_end + 1;
    }

    Some(result)
}

// ---------------------------------------------------------------------------
// Manifest -> Schema conversion
// ---------------------------------------------------------------------------

fn manifest_to_schema(manifest: &AppManifest) -> Schema {
    Schema {
        entities: manifest
            .entities
            .iter()
            .map(|e| Entity {
                name: e.name.clone(),
                fields: e
                    .fields
                    .iter()
                    .map(|f| Field {
                        name: f.name.clone(),
                        field_type: parse_field_type(&f.field_type),
                        optional: f.optional,
                        unique: f.unique,
                    })
                    .collect(),
                indexes: e
                    .indexes
                    .iter()
                    .map(|i| Index {
                        name: i.name.clone(),
                        fields: i.fields.clone(),
                        unique: i.unique,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn parse_field_type(s: &str) -> FieldType {
    match s {
        "string" => FieldType::String,
        "int" => FieldType::Int,
        "float" => FieldType::Float,
        "bool" => FieldType::Bool,
        "datetime" => FieldType::Datetime,
        "richtext" => FieldType::Richtext,
        other if other.starts_with("id(") && other.ends_with(')') => {
            FieldType::Id(other[3..other.len() - 1].to_string())
        }
        _ => FieldType::String,
    }
}

// ---------------------------------------------------------------------------
// Diagnostic output
// ---------------------------------------------------------------------------

fn print_diagnostics(diagnostics: &[Diagnostic], json_mode: bool) {
    if json_mode {
        print!("[");
        for (i, d) in diagnostics.iter().enumerate() {
            if i > 0 {
                print!(",");
            }
            let severity = format!("{}", d.severity);
            let hint = match &d.hint {
                Some(h) => format!(",\"hint\":\"{}\"", escape_json(h)),
                None => String::new(),
            };
            let span = match &d.span {
                Some(s) => {
                    let mut parts = format!("\"file\":\"{}\"", escape_json(&s.file));
                    if let Some(l) = s.line {
                        parts.push_str(&format!(",\"line\":{l}"));
                    }
                    if let Some(c) = s.column {
                        parts.push_str(&format!(",\"column\":{c}"));
                    }
                    format!(",\"span\":{{{parts}}}")
                }
                None => String::new(),
            };
            print!(
                "{{\"severity\":\"{severity}\",\"code\":\"{}\",\"message\":\"{}\"{hint}{span}}}",
                escape_json(&d.code),
                escape_json(&d.message)
            );
        }
        println!("]");
    } else {
        for d in diagnostics {
            match d.severity {
                Severity::Error => eprintln!("{d}"),
                _ => println!("{d}"),
            }
        }
    }
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn print_usage() {
    println!("agentdb <command>");
    println!();
    println!("Commands:");
    println!("  doctor [manifest-path]   Validate an app manifest");
    println!("  explain [manifest-path]  Print a structured summary");
    println!("  version                  Print version");
    println!();
    println!("Flags:");
    println!("  --json    Machine-readable JSON output");
}
