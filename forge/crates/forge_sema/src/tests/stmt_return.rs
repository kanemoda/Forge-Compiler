//! Tests for `return` statement checking.
//!
//! Covers the void vs non-void return matching:
//!
//! * `return;` is legal only when the function returns `void`.
//! * `return <expr>;` is legal only when the function does NOT return
//!   `void`.

use crate::scope::{ScopeKind, SymbolTable};
use crate::stmt::analyze_stmt;
use crate::{context::SemaContext, types::QualType};

use super::helpers::*;

fn drive_return(value: Option<forge_parser::ast::Expr>, ret_ty: QualType) -> SemaContext {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table.push_scope(ScopeKind::Function);
    let mut fnc = fn_ctx(ret_ty);
    let s = h_return(value);
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);
    ctx
}

#[test]
fn void_function_with_bare_return_accepts() {
    let ctx = drive_return(None, QualType::unqualified(void()));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn non_void_function_with_bare_return_is_error() {
    let ctx = drive_return(None, QualType::unqualified(int()));
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("non-void function must return a value")),
        "expected missing-value error, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn non_void_function_with_valued_return_accepts() {
    let ctx = drive_return(Some(h_int_lit(42)), QualType::unqualified(int()));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn void_function_with_valued_return_is_error() {
    let ctx = drive_return(Some(h_int_lit(42)), QualType::unqualified(void()));
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("void function cannot return a value")),
        "expected extra-value error, got {:?}",
        ctx.diagnostics
    );
}
