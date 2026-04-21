//! Ariadne-backed rendering of [`crate::Diagnostic`] values.

use crate::{render_diagnostics_to_string, Diagnostic};

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
