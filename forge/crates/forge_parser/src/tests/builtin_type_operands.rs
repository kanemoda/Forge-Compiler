//! AST-shape tests for `__builtin_offsetof` and
//! `__builtin_types_compatible_p`.
//!
//! These two builtins take **type-names** as arguments, so they cannot
//! parse as ordinary function calls.  The parser intercepts them in
//! `parse_prefix` and lowers each to a dedicated [`Expr`] variant
//! ([`Expr::BuiltinOffsetof`] / [`Expr::BuiltinTypesCompatibleP`]).
//!
//! The tests here verify the *syntactic* shape only — sema is
//! responsible for checking that the type resolves to a struct, that
//! members exist, and for computing the final integer result.

use crate::ast::{Expr, OffsetofMember};

use super::helpers::*;

// =========================================================================
// __builtin_offsetof — designator shapes
// =========================================================================

#[test]
fn offsetof_single_field_designator_parses() {
    let tu = parse_tu("struct S { int a; int b; }; int x = __builtin_offsetof(struct S, b);");
    let init = first_initializer_expr(&tu);
    match init {
        Expr::BuiltinOffsetof { designator, .. } => {
            assert_eq!(designator.len(), 1);
            match &designator[0] {
                OffsetofMember::Field(name) => assert_eq!(name, "b"),
                _ => panic!("expected Field('b'), got {:?}", designator[0]),
            }
        }
        other => panic!("expected BuiltinOffsetof, got {other:?}"),
    }
}

#[test]
fn offsetof_dotted_chain_parses_as_fields() {
    let tu = parse_tu(
        "struct Inner { int x; int y; };\n\
         struct Outer { struct Inner head; struct Inner tail; };\n\
         int o = __builtin_offsetof(struct Outer, tail.y);",
    );
    let init = first_initializer_expr(&tu);
    match init {
        Expr::BuiltinOffsetof { designator, .. } => {
            assert_eq!(designator.len(), 2);
            match (&designator[0], &designator[1]) {
                (OffsetofMember::Field(a), OffsetofMember::Field(b)) => {
                    assert_eq!(a, "tail");
                    assert_eq!(b, "y");
                }
                other => panic!("expected [Field, Field], got {other:?}"),
            }
        }
        other => panic!("expected BuiltinOffsetof, got {other:?}"),
    }
}

#[test]
fn offsetof_subscript_designator_parses() {
    let tu = parse_tu(
        "struct A { int arr[10]; };\n\
         int o = __builtin_offsetof(struct A, arr[3]);",
    );
    let init = first_initializer_expr(&tu);
    match init {
        Expr::BuiltinOffsetof { designator, .. } => {
            assert_eq!(designator.len(), 2);
            match (&designator[0], &designator[1]) {
                (OffsetofMember::Field(name), OffsetofMember::Subscript(idx)) => {
                    assert_eq!(name, "arr");
                    match idx.as_ref() {
                        Expr::IntLiteral { value, .. } => assert_eq!(*value, 3),
                        other => panic!("expected IntLiteral(3), got {other:?}"),
                    }
                }
                other => panic!("expected [Field, Subscript], got {other:?}"),
            }
        }
        other => panic!("expected BuiltinOffsetof, got {other:?}"),
    }
}

#[test]
fn offsetof_mixed_chain_parses() {
    // `a.arr[3].b` should yield Field,Subscript,Field.
    let tu = parse_tu(
        "struct Leaf { int b; };\n\
         struct Mid { struct Leaf arr[4]; };\n\
         struct Outer { struct Mid a; };\n\
         int o = __builtin_offsetof(struct Outer, a.arr[3].b);",
    );
    let init = first_initializer_expr(&tu);
    match init {
        Expr::BuiltinOffsetof { designator, .. } => {
            assert_eq!(designator.len(), 4);
            assert!(matches!(&designator[0], OffsetofMember::Field(n) if n == "a"));
            assert!(matches!(&designator[1], OffsetofMember::Field(n) if n == "arr"));
            assert!(matches!(&designator[2], OffsetofMember::Subscript(_)));
            assert!(matches!(&designator[3], OffsetofMember::Field(n) if n == "b"));
        }
        other => panic!("expected BuiltinOffsetof, got {other:?}"),
    }
}

// =========================================================================
// __builtin_types_compatible_p — type-operand shapes
// =========================================================================

#[test]
fn types_compatible_p_two_primitive_operands_parses() {
    let tu = parse_tu("int x = __builtin_types_compatible_p(int, long);");
    let init = first_initializer_expr(&tu);
    assert!(matches!(init, Expr::BuiltinTypesCompatibleP { .. }));
}

#[test]
fn types_compatible_p_pointer_with_qualifiers_parses() {
    // `int *` vs `const int *` — both operands are pointer type-names
    // with differing qualifiers.  The parser just captures both; sema
    // decides compatibility.
    let tu = parse_tu("int x = __builtin_types_compatible_p(int *, const int *);");
    let init = first_initializer_expr(&tu);
    assert!(matches!(init, Expr::BuiltinTypesCompatibleP { .. }));
}

// =========================================================================
// Parse-error cases
// =========================================================================

#[test]
fn offsetof_with_no_arguments_is_a_parse_error() {
    let (_tu, diags) = parse_tu_with_diagnostics("int x = __builtin_offsetof();");
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == forge_diagnostics::Severity::Error)
        .collect();
    assert!(
        !errors.is_empty(),
        "expected at least one error diagnostic, got none"
    );
}

#[test]
fn offsetof_without_designator_is_a_parse_error() {
    // `__builtin_offsetof(int)` — missing comma + designator.  Parser
    // demands a comma-separated second operand.
    let (_tu, diags) = parse_tu_with_diagnostics("int x = __builtin_offsetof(int);");
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == forge_diagnostics::Severity::Error)
        .collect();
    assert!(
        !errors.is_empty(),
        "expected at least one error diagnostic, got none"
    );
}

// =========================================================================
// Helpers
// =========================================================================

fn first_initializer_expr(tu: &crate::ast::TranslationUnit) -> &Expr {
    // Walk past any leading struct/union tag declarations to find the
    // first top-level `int x = ...;` and return the RHS.
    for ext in &tu.declarations {
        if let crate::ast::ExternalDeclaration::Declaration(d) = ext {
            if let Some(init) = d.init_declarators.first() {
                if let Some(crate::ast::Initializer::Expr(e)) = &init.initializer {
                    return e.as_ref();
                }
            }
        }
    }
    panic!("no init-declarator expression found in translation unit");
}
