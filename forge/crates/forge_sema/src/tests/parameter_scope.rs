//! Regression tests for the C17 §6.2.1p4 function-definition-scope rule.
//!
//! Parameters of a function definition share their scope with the
//! outermost compound of the body — the body does NOT start a fresh
//! block.  In concrete terms:
//!
//! * a body-local `int a;` with the same name as a parameter `a` must be
//!   a *redefinition*, not a shadow; and
//! * a `return a;` inside the body must see the parameter `a`.

use crate::scope::SymbolTable;
use crate::stmt::analyze_function_def;
use crate::{context::SemaContext, types::Type};

use super::helpers::*;

#[test]
fn parameter_is_visible_inside_body() {
    // int f(int a) { return a; }
    let fd = h_fn_def(
        h_int_specs(),
        h_func_decl_int_params("f", &["a"]),
        h_compound(vec![h_bstmt(h_return(Some(h_ident_expr("a"))))]),
    );
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    analyze_function_def(&fd, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn body_local_with_same_name_as_parameter_is_redefinition() {
    // int f(int a) { int a; return 0; }
    //
    // Per C17 §6.2.1p4 the outermost body compound does NOT open a new
    // scope — the body-local `a` collides with the parameter `a`.
    let body_decl = h_declaration(h_int_specs(), vec![h_init_decl(h_ident_decl("a"), None)]);
    let fd = h_fn_def(
        h_int_specs(),
        h_func_decl_int_params("f", &["a"]),
        h_compound(vec![
            h_bdecl(body_decl),
            h_bstmt(h_return(Some(h_int_lit(0)))),
        ]),
    );
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    analyze_function_def(&fd, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.has_errors(),
        "expected redefinition error, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn nested_block_can_shadow_parameter() {
    // int f(int a) { { int a = 0; } return a; }
    //
    // A *nested* compound does push a fresh block scope, so shadowing is
    // legal there.
    let inner_decl = h_declaration(
        h_int_specs(),
        vec![h_init_decl(h_ident_decl("a"), Some(h_expr_init(0)))],
    );
    let inner_block = h_compound_stmt(vec![h_bdecl(inner_decl)]);
    let fd = h_fn_def(
        h_int_specs(),
        h_func_decl_int_params("f", &["a"]),
        h_compound(vec![
            h_bstmt(inner_block),
            h_bstmt(h_return(Some(h_ident_expr("a")))),
        ]),
    );
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    analyze_function_def(&fd, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn parameter_ty_is_int_by_default() {
    // Sanity check that `h_func_decl_int_params` produces `int` parameters.
    let fd = h_fn_def(
        h_int_specs(),
        h_func_decl_int_params("f", &["x"]),
        h_compound(vec![h_bstmt(h_return(Some(h_ident_expr("x"))))]),
    );
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    analyze_function_def(&fd, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let sym = table.lookup("f").expect("f must be declared");
    match &sym.ty.ty {
        Type::Function { params, .. } => {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name.as_deref(), Some("x"));
            assert!(matches!(params[0].ty.ty, Type::Int { is_unsigned: false }));
        }
        other => panic!("expected function type, got {other:?}"),
    }
}
