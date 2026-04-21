//! Ariadne-backed rendering of [`crate::Diagnostic`] values.

use crate::{render_diagnostics_to_string, Diagnostic, ExpansionTable, SourceMap, Span};

fn sm(name: &str, src: &str) -> SourceMap {
    let mut sm = SourceMap::new();
    sm.add_file(name.to_string(), src.to_string());
    sm
}

fn empty_expansions() -> ExpansionTable {
    ExpansionTable::new()
}

/// Core demonstration test: a C source with a missing semicolon produces
/// a rendered diagnostic that contains the error message text.
#[test]
fn test_render_missing_semicolon() {
    let source = "int main() { return 0 }";
    let source_map = sm("test.c", source);

    // '0' is at byte 20; the expected ';' would sit at byte 21.
    let diag = Diagnostic::error("expected ';' after return statement")
        .span(Span::primary(21, 22))
        .label("expected ';' here")
        .note("every statement in C must end with a semicolon");

    let rendered = render_diagnostics_to_string(&source_map, &empty_expansions(), &[diag]);

    assert!(
        rendered.contains("expected ';' after return statement"),
        "error message not found in rendered output:\n{rendered}"
    );
    assert!(
        rendered.contains("expected ';' here"),
        "label text not found in rendered output:\n{rendered}"
    );
    assert!(
        rendered.contains("every statement in C must end with a semicolon"),
        "note text not found in rendered output:\n{rendered}"
    );
    assert!(
        rendered.contains("test.c"),
        "filename not found in rendered output:\n{rendered}"
    );
}

#[test]
fn test_render_to_string_is_plain_text() {
    let source = "int x = ;";
    let source_map = sm("test.c", source);
    let diag = Diagnostic::error("expected expression").span(Span::primary(8, 9));
    let rendered = render_diagnostics_to_string(&source_map, &empty_expansions(), &[diag]);

    assert!(
        !rendered.contains('\x1b'),
        "unexpected ANSI escape code in plain-text output:\n{rendered}"
    );
}

#[test]
fn test_render_multiple_diagnostics() {
    let source = "int x = ; int y = ;";
    let source_map = sm("multi.c", source);
    let diagnostics = vec![
        Diagnostic::error("expected expression after '='")
            .span(Span::primary(8, 9))
            .label("expected expression here"),
        Diagnostic::error("expected expression after '='")
            .span(Span::primary(18, 19))
            .label("expected expression here"),
    ];
    let rendered = render_diagnostics_to_string(&source_map, &empty_expansions(), &diagnostics);
    assert!(
        rendered.matches("expected expression after '='").count() >= 2,
        "expected at least two occurrences of the error message:\n{rendered}"
    );
}
