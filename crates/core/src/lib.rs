use std::fmt;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub file: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

// ---------------------------------------------------------------------------
// Diagnostic — structured, machine-readable error/warning
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub span: Option<Span>,
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
// DiagnosticResult — envelope for operations that produce diagnostics
// ---------------------------------------------------------------------------

pub struct DiagnosticResult<T> {
    pub value: Option<T>,
    pub diagnostics: Vec<Diagnostic>,
}

impl<T> DiagnosticResult<T> {
    pub fn ok(value: T) -> Self {
        Self {
            value: Some(value),
            diagnostics: Vec::new(),
        }
    }

    pub fn with_diagnostics(value: T, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            value: Some(value),
            diagnostics,
        }
    }

    pub fn fail(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            value: None,
            diagnostics,
        }
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}

// ---------------------------------------------------------------------------
// AppManifest — skeleton for the canonical manifest shape
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppManifest {
    pub name: String,
    pub version: String,
    pub entities: Vec<ManifestEntity>,
    pub routes: Vec<ManifestRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntity {
    pub name: String,
    pub fields: Vec<ManifestField>,
    pub indexes: Vec<ManifestIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestField {
    pub name: String,
    pub field_type: String,
    pub optional: bool,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestIndex {
    pub name: String,
    pub fields: Vec<String>,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestRoute {
    pub path: String,
    pub mode: String,
}
