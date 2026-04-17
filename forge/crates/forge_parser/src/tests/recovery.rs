//! Tests for Prompt 3.5 — panic-mode error recovery.
//!
//! These tests all supply intentionally-broken input.  We assert that:
//!   * at least one error diagnostic is produced, and
//!   * the parser still recovers far enough to parse the *following*
//!     well-formed constructs.

use forge_diagnostics::Severity;

use crate::ast::*;
use crate::decl::declarator_name;

use super::helpers::parse_tu_with_diagnostics;

/// Count error-severity diagnostics in `diags`.
fn error_count(diags: &[forge_diagnostics::Diagnostic]) -> usize {
    diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count()
}

fn as_fn_def(decl: &ExternalDeclaration) -> &FunctionDef {
    match decl {
        ExternalDeclaration::FunctionDef(f) => f,
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

// =========================================================================
// Recovery at file scope
// =========================================================================

#[test]
fn recovers_to_next_semicolon_at_file_scope() {
    // The first line is garbage.  After synchronize, the parser should
    // pick up `int y;` and parse it successfully.
    let (tu, diags) = parse_tu_with_diagnostics("garbage @ broken; int y;");
    assert!(error_count(&diags) > 0, "expected at least one error");

    // Find the surviving `int y;` among the recovered declarations.
    let found_y = tu.declarations.iter().any(|ed| match ed {
        ExternalDeclaration::Declaration(d) => d
            .init_declarators
            .iter()
            .any(|id| declarator_name(&id.declarator) == Some("y")),
        _ => false,
    });
    assert!(found_y, "expected to recover and parse `int y;`");
}

#[test]
fn recovers_at_declaration_start() {
    // `@@@` is junk; `int` afterwards should still parse.
    let (tu, diags) = parse_tu_with_diagnostics("@@@ int x;");
    assert!(error_count(&diags) > 0);
    let found_x = tu.declarations.iter().any(|ed| match ed {
        ExternalDeclaration::Declaration(d) => d
            .init_declarators
            .iter()
            .any(|id| declarator_name(&id.declarator) == Some("x")),
        _ => false,
    });
    assert!(found_x, "expected to recover and parse `int x;`");
}

#[test]
fn does_not_infinite_loop_on_stream_of_junk() {
    // The parser must terminate even when every token is unexpected.
    let (_tu, diags) = parse_tu_with_diagnostics("@ @ @ @ @");
    // At least one error, and we returned (didn't hang).
    assert!(error_count(&diags) > 0);
}

// =========================================================================
// Recovery inside function bodies
// =========================================================================

#[test]
fn bad_statement_recovers_to_next_semicolon() {
    // `broken @ stuff;` inside the body produces errors, but the parser
    // should still parse the subsequent `int y = 2;` declaration.
    let src = "int f(void) { int x = 1; broken @ stuff; int y = 2; return y; }";
    let (tu, diags) = parse_tu_with_diagnostics(src);
    assert!(error_count(&diags) > 0, "expected recovery to emit errors");
    assert_eq!(tu.declarations.len(), 1);
    let f = as_fn_def(&tu.declarations[0]);
    let has_y = f.body.items.iter().any(|item| match item {
        BlockItem::Declaration(d) => d
            .init_declarators
            .iter()
            .any(|id| declarator_name(&id.declarator) == Some("y")),
        _ => false,
    });
    assert!(has_y, "expected `int y = 2;` to survive recovery");
}

#[test]
fn missing_semicolon_reports_error_and_continues() {
    // `int x = 1` without `;` should diagnose but still parse `int y;`.
    let src = "int x = 1 int y;";
    let (tu, diags) = parse_tu_with_diagnostics(src);
    assert!(error_count(&diags) > 0);
    // At least one of the recovered declarations names `y`.
    let has_y = tu.declarations.iter().any(|ed| {
        if let ExternalDeclaration::Declaration(d) = ed {
            d.init_declarators
                .iter()
                .any(|id| declarator_name(&id.declarator) == Some("y"))
        } else {
            false
        }
    });
    assert!(has_y);
}

// =========================================================================
// Declarations following broken ones
// =========================================================================

#[test]
fn following_declaration_survives() {
    // `int = 3;` is invalid (no declarator), but `int y = 4;` should
    // still be parsed afterwards.
    let src = "int = 3; int y = 4;";
    let (tu, diags) = parse_tu_with_diagnostics(src);
    assert!(error_count(&diags) > 0);
    let has_y = tu.declarations.iter().any(|ed| {
        matches!(
            ed,
            ExternalDeclaration::Declaration(d)
                if d.init_declarators.iter().any(|id| declarator_name(&id.declarator) == Some("y"))
        )
    });
    assert!(has_y);
}
