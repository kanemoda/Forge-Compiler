//! Fluent builder API on [`crate::Diagnostic`].

use crate::{Diagnostic, Severity, Span};

#[test]
fn test_error_builder_defaults() {
    let diag = Diagnostic::error("something went wrong");
    assert_eq!(diag.severity, Severity::Error);
    assert_eq!(diag.message, "something went wrong");
    assert_eq!(diag.span.start, 0);
    assert_eq!(diag.span.end, 0);
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
        .span(Span::primary(5, 6))
        .label("expected ';' here")
        .note("every statement in C must end with a semicolon");

    assert_eq!(diag.span, Span::primary(5, 6));
    assert_eq!(diag.labels.len(), 1);
    assert_eq!(diag.labels[0].span, Span::primary(5, 6));
    assert_eq!(diag.labels[0].message, "expected ';' here");
    assert_eq!(diag.notes.len(), 1);
}

#[test]
fn test_label_at_uses_explicit_span() {
    let diag = Diagnostic::error("type mismatch")
        .span(Span::primary(10, 15))
        .label_at(Span::primary(0, 4), "declared as 'int' here")
        .label("used as 'char' here");

    assert_eq!(diag.labels[0].span, Span::primary(0, 4));
    assert_eq!(diag.labels[1].span, Span::primary(10, 15));
}
