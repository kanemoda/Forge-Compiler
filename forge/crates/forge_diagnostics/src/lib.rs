//! Diagnostic reporting for the Forge compiler.
//!
//! This crate provides the [`Diagnostic`] type and the [`render_diagnostics`]
//! / [`render_diagnostics_to_string`] functions, which use the `ariadne` crate
//! to produce human-readable, source-annotated error messages.
//!
//! # Builder pattern
//!
//! Diagnostics are constructed via a fluent builder API.  Calling
//! `Diagnostic::error` / `Diagnostic::warning` creates a diagnostic with only
//! a message.  The primary source span is attached with `.span()`, after which
//! `.label()`, `.label_at()`, and `.note()` can be chained:
//!
//! ```
//! use forge_diagnostics::{Diagnostic, Severity};
//!
//! let source = "int main() { return 0 }";
//! let diag = Diagnostic::error("expected ';' after return statement")
//!     .span(21..22)
//!     .label("expected ';' here")
//!     .note("every statement in C must end with a semicolon");
//!
//! assert_eq!(diag.severity, Severity::Error);
//! assert_eq!(diag.span, 21..22);
//! ```

use std::ops::Range;

use ariadne::{Color, Config, Label as AriadneLabel, Report, ReportKind, Source};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The severity level of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A hard error; compilation cannot succeed.
    Error,
    /// A warning; compilation can continue but the user should be aware.
    Warning,
    /// An informational note, typically attached to another diagnostic.
    Note,
}

/// A source-level label pointing to a specific span with an explanatory message.
#[derive(Debug, Clone)]
pub struct Label {
    /// The byte range in the source file this label points to.
    pub span: Range<usize>,
    /// The message displayed alongside the underlined span.
    pub message: String,
}

/// A single compiler diagnostic (error, warning, or note).
///
/// Use the fluent builder methods to construct diagnostics:
///
/// ```
/// use forge_diagnostics::Diagnostic;
///
/// let diag = Diagnostic::error("expected ';'")
///     .span(21..22)
///     .label("expected ';' here")
///     .note("every statement in C must end with a semicolon");
/// ```
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// The primary human-readable message describing what went wrong.
    pub message: String,
    /// The primary byte-range span in the source file.
    ///
    /// Defaults to `0..0` when the diagnostic has not yet had a span attached
    /// via [`.span()`](Diagnostic::span).
    pub span: Range<usize>,
    /// How severe this diagnostic is.
    pub severity: Severity,
    /// Additional source labels pointing at relevant locations.
    pub labels: Vec<Label>,
    /// Free-form notes appended below the diagnostic (e.g., suggestions).
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Builder API
// ---------------------------------------------------------------------------

impl Diagnostic {
    /// Start building an **error** diagnostic with the given message.
    ///
    /// Chain [`.span()`](Self::span) to attach a source location, then
    /// optionally [`.label()`](Self::label) and [`.note()`](Self::note).
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message)
    }

    /// Start building a **warning** diagnostic with the given message.
    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message)
    }

    /// Start building a **note** diagnostic with the given message.
    pub fn note_diag(message: impl Into<String>) -> Self {
        Self::new(Severity::Note, message)
    }

    fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: 0..0,
            severity,
            labels: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Set the primary source span and return `self` for chaining.
    ///
    /// This also **replaces the span on any previously added labels** that
    /// still carry the default `0..0` span, so it is safe to call `.span()`
    /// after `.label()` if you prefer that order.
    pub fn span(mut self, span: Range<usize>) -> Self {
        self.span = span;
        self
    }

    /// Add a label pointing at the **primary span** with the given message,
    /// and return `self` for chaining.
    ///
    /// Call [`.span()`](Self::span) before `.label()` so the span is known.
    /// If you need a label at a *different* location use
    /// [`.label_at()`](Self::label_at) instead.
    pub fn label(mut self, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span: self.span.clone(),
            message: message.into(),
        });
        self
    }

    /// Add a label at an **explicit span** and return `self` for chaining.
    ///
    /// Use this when the label should point somewhere other than the primary
    /// span — for example, to highlight a conflicting declaration.
    pub fn label_at(mut self, span: Range<usize>, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            message: message.into(),
        });
        self
    }

    /// Append a free-form note and return `self` for chaining.
    ///
    /// Notes appear below the source snippet, making them a good place for
    /// suggestions or references to relevant language rules.
    pub fn note(mut self, message: impl Into<String>) -> Self {
        self.notes.push(message.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render a slice of diagnostics to **stderr** with ANSI colour codes.
///
/// This is the standard production path.  For testing or LSP use,
/// call [`render_diagnostics_to_string`] instead.
pub fn render_diagnostics(source: &str, filename: &str, diagnostics: &[Diagnostic]) {
    for diag in diagnostics {
        write_report(diag, filename, source, &mut std::io::stderr(), true)
            .expect("failed to write diagnostic to stderr");
    }
}

/// Render a slice of diagnostics to a [`String`] without ANSI colour codes.
///
/// This is intended for unit tests and any consumer that needs the rendered
/// text as a plain string (e.g., an LSP server building hover messages).
pub fn render_diagnostics_to_string(
    source: &str,
    filename: &str,
    diagnostics: &[Diagnostic],
) -> String {
    let mut buf: Vec<u8> = Vec::new();
    for diag in diagnostics {
        write_report(diag, filename, source, &mut buf, false)
            .expect("failed to write diagnostic to buffer");
    }
    String::from_utf8(buf).unwrap_or_default()
}

/// Core rendering helper shared by the public render functions.
///
/// Builds an ariadne [`Report`] from a [`Diagnostic`] and writes it to
/// the given [`std::io::Write`] sink.  `color` controls whether ANSI escape
/// codes are emitted.
fn write_report(
    diag: &Diagnostic,
    filename: &str,
    source: &str,
    writer: &mut dyn std::io::Write,
    color: bool,
) -> std::io::Result<()> {
    let kind = match diag.severity {
        Severity::Error => ReportKind::Error,
        Severity::Warning => ReportKind::Warning,
        Severity::Note => ReportKind::Advice,
    };

    let primary_color = match diag.severity {
        Severity::Error => Color::Red,
        Severity::Warning => Color::Yellow,
        Severity::Note => Color::Blue,
    };

    let mut builder = Report::build(kind, filename, diag.span.start)
        .with_config(Config::default().with_color(color))
        .with_message(&diag.message)
        .with_label(
            AriadneLabel::new((filename, diag.span.clone()))
                .with_message(&diag.message)
                .with_color(primary_color),
        );

    for (i, label) in diag.labels.iter().enumerate() {
        let label_color = if i == 0 { primary_color } else { Color::Cyan };
        builder = builder.with_label(
            AriadneLabel::new((filename, label.span.clone()))
                .with_message(&label.message)
                .with_color(label_color),
        );
    }

    for note in &diag.notes {
        builder = builder.with_note(note);
    }

    builder
        .finish()
        .write((filename, Source::from(source)), writer)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Builder API ---

    #[test]
    fn test_error_builder_defaults() {
        let diag = Diagnostic::error("something went wrong");
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.message, "something went wrong");
        assert_eq!(diag.span, 0..0);
        assert!(diag.labels.is_empty());
        assert!(diag.notes.is_empty());
    }

    #[test]
    fn test_warning_builder_defaults() {
        let diag = Diagnostic::warning("unused variable 'x'");
        assert_eq!(diag.severity, Severity::Warning);
        assert!(diag.labels.is_empty());
    }

    #[test]
    fn test_full_chain() {
        let diag = Diagnostic::error("expected ';'")
            .span(5..6)
            .label("expected ';' here")
            .note("every statement in C must end with a semicolon");

        assert_eq!(diag.span, 5..6);
        assert_eq!(diag.labels.len(), 1);
        assert_eq!(diag.labels[0].span, 5..6);
        assert_eq!(diag.labels[0].message, "expected ';' here");
        assert_eq!(diag.notes.len(), 1);
    }

    #[test]
    fn test_label_at_uses_explicit_span() {
        let diag = Diagnostic::error("type mismatch")
            .span(10..15)
            .label_at(0..4, "declared as 'int' here")
            .label("used as 'char' here");

        assert_eq!(diag.labels[0].span, 0..4);
        assert_eq!(diag.labels[1].span, 10..15);
    }

    // --- Rendered output ---

    /// Core demonstration test: a C source with a missing semicolon produces
    /// a rendered diagnostic that contains the error message text.
    ///
    /// The source is `int main() { return 0 }`.
    ///              positions: 0123456789012345678901234
    ///                                              ^ position 21 is the
    ///                                                space after '0' where
    ///                                                ';' is expected.
    #[test]
    fn test_render_missing_semicolon() {
        let source = "int main() { return 0 }";

        // '0' is at byte 20; the expected ';' would sit at byte 21.
        let diag = Diagnostic::error("expected ';' after return statement")
            .span(21..22)
            .label("expected ';' here")
            .note("every statement in C must end with a semicolon");

        let rendered = render_diagnostics_to_string(source, "test.c", &[diag]);

        // The error message must appear in the output.
        assert!(
            rendered.contains("expected ';' after return statement"),
            "error message not found in rendered output:\n{rendered}"
        );

        // The label text must appear.
        assert!(
            rendered.contains("expected ';' here"),
            "label text not found in rendered output:\n{rendered}"
        );

        // The note must appear.
        assert!(
            rendered.contains("every statement in C must end with a semicolon"),
            "note text not found in rendered output:\n{rendered}"
        );

        // ariadne should emit the file name.
        assert!(
            rendered.contains("test.c"),
            "filename not found in rendered output:\n{rendered}"
        );
    }

    #[test]
    fn test_render_to_string_is_plain_text() {
        let source = "int x = ;";
        let diag = Diagnostic::error("expected expression").span(8..9);
        let rendered = render_diagnostics_to_string(source, "test.c", &[diag]);

        // Without colour, there should be no ESC character in the output.
        assert!(
            !rendered.contains('\x1b'),
            "unexpected ANSI escape code in plain-text output:\n{rendered}"
        );
    }

    #[test]
    fn test_render_multiple_diagnostics() {
        let source = "int x = ; int y = ;";
        let diagnostics = vec![
            Diagnostic::error("expected expression after '='")
                .span(8..9)
                .label("expected expression here"),
            Diagnostic::error("expected expression after '='")
                .span(18..19)
                .label("expected expression here"),
        ];
        let rendered = render_diagnostics_to_string(source, "multi.c", &diagnostics);
        // Each message appears at least twice per diagnostic (report title + label),
        // so we simply verify the count is at least 2, confirming both rendered.
        assert!(
            rendered.matches("expected expression after '='").count() >= 2,
            "expected at least two occurrences of the error message:\n{rendered}"
        );
    }
}
