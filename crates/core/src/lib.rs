use std::fmt;

use serde::{Deserialize, Serialize};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Ok = 0,
    Error = 1,
    Usage = 64,
    Unavailable = 69,
}

impl ExitCode {
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

// ---------------------------------------------------------------------------
// Severity & Span — shared diagnostic primitives
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => f.write_str("error"),
            Severity::Warning => f.write_str("warning"),
            Severity::Info => f.write_str("info"),
        }
    }
}

/// Optional source location for a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

// ---------------------------------------------------------------------------
// Diagnostic — structured, machine-readable error/warning
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.severity, self.code, self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, " (hint: {hint})")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// AppManifest — canonical manifest shape
// ---------------------------------------------------------------------------

pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppManifest {
    pub manifest_version: u32,
    pub name: String,
    pub version: String,
    pub entities: Vec<ManifestEntity>,
    pub routes: Vec<ManifestRoute>,
    #[serde(default)]
    pub queries: Vec<ManifestQuery>,
    #[serde(default)]
    pub actions: Vec<ManifestAction>,
    #[serde(default)]
    pub policies: Vec<ManifestPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntity {
    pub name: String,
    pub fields: Vec<ManifestField>,
    pub indexes: Vec<ManifestIndex>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<ManifestRelation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRelation {
    pub name: String,
    pub target: String,
    pub field: String,
    #[serde(default)]
    pub many: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub optional: bool,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestIndex {
    pub name: String,
    pub fields: Vec<String>,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestRoute {
    pub path: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestQuery {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<ManifestField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestAction {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<ManifestField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestPolicy {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    pub allow: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_values() {
        assert_eq!(ExitCode::Ok.as_i32(), 0);
        assert_eq!(ExitCode::Error.as_i32(), 1);
        assert_eq!(ExitCode::Usage.as_i32(), 64);
        assert_eq!(ExitCode::Unavailable.as_i32(), 69);
    }

    #[test]
    fn severity_display() {
        assert_eq!(format!("{}", Severity::Error), "error");
        assert_eq!(format!("{}", Severity::Warning), "warning");
        assert_eq!(format!("{}", Severity::Info), "info");
    }

    #[test]
    fn diagnostic_display_without_hint() {
        let d = Diagnostic {
            severity: Severity::Error,
            code: "TEST".into(),
            message: "something failed".into(),
            span: None,
            hint: None,
        };
        assert_eq!(format!("{d}"), "[error] TEST: something failed");
    }

    #[test]
    fn diagnostic_display_with_hint() {
        let d = Diagnostic {
            severity: Severity::Warning,
            code: "WARN".into(),
            message: "check this".into(),
            span: None,
            hint: Some("try again".into()),
        };
        assert_eq!(format!("{d}"), "[warning] WARN: check this (hint: try again)");
    }

    #[test]
    fn manifest_version_constant() {
        assert_eq!(MANIFEST_VERSION, 1);
    }
}
