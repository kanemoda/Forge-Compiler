//! Tests for [`analyze_function_def`] — the whole-function entry point.
//!
//! Covers the basic flow (symbol registration, `is_defined`, linkage),
//! the non-void-returns warning heuristic, the `_Noreturn` exemption,
//! and redefinition diagnostics.

use forge_parser::ast::FunctionSpecifier;

use crate::scope::{Linkage, Scope, ScopeKind, SymbolKind, SymbolTable};
use crate::stmt::analyze_function_def;
use crate::{context::SemaContext, types::Type};

use super::helpers::*;

fn run(func: forge_parser::ast::FunctionDef) -> (SemaContext, SymbolTable) {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    analyze_function_def(&func, &mut table, &ti(), &mut ctx);
    (ctx, table)
}

#[test]
fn function_definition_registers_symbol_defined() {
    let (ctx, table) = run(h_fn_int_void(
        "f",
        vec![h_bstmt(h_return(Some(h_int_lit(0))))],
    ));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    let sym = table.lookup("f").expect("function symbol must exist");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert!(sym.is_defined);
    assert_eq!(sym.linkage, Linkage::External);
    assert!(matches!(sym.ty.ty, Type::Function { .. }));
}

#[test]
fn function_body_pops_its_function_scope() {
    // After analysis only the file scope must remain — a leaked
    // push/pop would leave function scope on the stack.
    let (_ctx, table) = run(h_fn_void_void("f", Vec::new()));
    assert_eq!(table.scope_depth(), 1);
    assert_eq!(table.current_scope_kind(), ScopeKind::File);
    let _: &Scope = table.current_scope(); // suppress dead-import warnings
}

#[test]
fn non_void_function_without_return_emits_warning() {
    // `int f(void) { }` — warns because the function returns `int` but
    // never executes a `return`.
    let (ctx, _table) = run(h_fn_int_void("f", Vec::new()));
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("does not return a value on any path")),
        "expected missing-return warning, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn non_void_function_with_return_is_silent() {
    let (ctx, _table) = run(h_fn_int_void(
        "f",
        vec![h_bstmt(h_return(Some(h_int_lit(0))))],
    ));
    assert!(
        !ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("does not return a value on any path")),
        "unexpected missing-return warning, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn void_function_without_return_is_silent() {
    let (ctx, _table) = run(h_fn_void_void("f", Vec::new()));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    assert!(
        !ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("does not return a value on any path")),
        "unexpected missing-return warning, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn noreturn_function_without_return_is_silent() {
    // `_Noreturn void die(void) { }` — legal, no warning.
    let fd = h_fn_def(
        h_specs_fnspec(
            vec![forge_parser::ast::TypeSpecifierToken::Void],
            vec![FunctionSpecifier::Noreturn],
        ),
        h_func_decl_void("die"),
        h_compound(Vec::new()),
    );
    let (ctx, table) = run(fd);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    let sym = table.lookup("die").expect("die must be declared");
    assert!(sym.is_noreturn);
}

#[test]
fn redefining_function_emits_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let f1 = h_fn_int_void("f", vec![h_bstmt(h_return(Some(h_int_lit(0))))]);
    analyze_function_def(&f1, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "first def should be clean");

    let f2 = h_fn_int_void("f", vec![h_bstmt(h_return(Some(h_int_lit(1))))]);
    analyze_function_def(&f2, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.has_errors(),
        "expected an error for redefinition, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn function_with_parameters_registers_them_in_function_scope() {
    // `int f(int a, int b) { return 0; }`
    let fd = h_fn_def(
        h_int_specs(),
        h_func_decl_int_params("f", &["a", "b"]),
        h_compound(vec![h_bstmt(h_return(Some(h_int_lit(0))))]),
    );
    let (ctx, table) = run(fd);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    // After analysis the parameters are out of scope — scope was popped.
    assert!(table.lookup("a").is_none());
    assert!(table.lookup("b").is_none());
    // But the function itself remains.
    assert!(table.lookup("f").is_some());
}

#[test]
fn duplicate_parameter_name_is_error() {
    // `int f(int a, int a) { return 0; }`
    let fd = h_fn_def(
        h_int_specs(),
        h_func_decl_int_params("f", &["a", "a"]),
        h_compound(vec![h_bstmt(h_return(Some(h_int_lit(0))))]),
    );
    let (ctx, _table) = run(fd);
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("redefinition of parameter 'a'")),
        "expected dup-param error, got {:?}",
        ctx.diagnostics
    );
}
