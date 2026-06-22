//! Diagnostics: the single source of truth for findings, rendered either as
//! rustc-style human output or as JSONL for tooling.

use std::fmt::Write as _;

use serde::Serialize;

use crate::ir::{JavaLoc, SrcLoc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java: Option<JavaLoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rust: Option<SrcLoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn new(severity: Severity, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity,
            code: code.into(),
            message: message.into(),
            java: None,
            rust: None,
            expected: None,
            found: None,
            notes: Vec::new(),
            help: None,
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, code, message)
    }

    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, message)
    }

    pub fn with_java(mut self, java: Option<JavaLoc>) -> Self {
        self.java = java;
        self
    }

    pub fn with_rust(mut self, rust: Option<SrcLoc>) -> Self {
        self.rust = rust;
        self
    }

    pub fn expected_found(mut self, expected: impl Into<String>, found: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self.found = Some(found.into());
        self
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
}

impl Report {
    pub fn push(&mut self, d: Diagnostic) {
        self.diagnostics.push(d);
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    pub fn has_errors(&self) -> bool {
        self.error_count() > 0
    }

    /// `true` if any diagnostic carries the given code (handy for tests).
    pub fn has_code(&self, code: &str) -> bool {
        self.diagnostics.iter().any(|d| d.code == code)
    }

    /// rustc-style rendering.
    pub fn render_human(&self) -> String {
        let mut out = String::new();
        for d in &self.diagnostics {
            let sev = match d.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            let _ = writeln!(out, "{sev}[{}]: {}", d.code, d.message);
            if let Some(j) = &d.java {
                let _ = writeln!(
                    out,
                    "  --> java: {}.{}{}",
                    j.class.replace('/', "."),
                    j.method,
                    j.descriptor
                );
            }
            if let Some(r) = &d.rust {
                let line = r.line.map(|l| format!(":{l}")).unwrap_or_default();
                let _ = writeln!(
                    out,
                    "  --> rust: {} ({}{})",
                    r.symbol,
                    r.file.display(),
                    line
                );
            }
            if let (Some(e), Some(f)) = (&d.expected, &d.found) {
                let _ = writeln!(out, "   = note: expected {e}");
                let _ = writeln!(out, "   = note:    found {f}");
            }
            for n in &d.notes {
                let _ = writeln!(out, "   = note: {n}");
            }
            if let Some(h) = &d.help {
                let _ = writeln!(out, "   = help: {h}");
            }
        }
        let _ = writeln!(
            out,
            "{} error(s), {} warning(s)",
            self.error_count(),
            self.warning_count()
        );
        out
    }

    /// One JSON object per line, closed by a summary object.
    pub fn render_json(&self) -> String {
        let mut out = String::new();
        for d in &self.diagnostics {
            let _ = writeln!(out, "{}", serde_json::to_string(d).unwrap());
        }
        let summary = serde_json::json!({
            "summary": { "errors": self.error_count(), "warnings": self.warning_count() }
        });
        let _ = writeln!(out, "{summary}");
        out
    }
}
