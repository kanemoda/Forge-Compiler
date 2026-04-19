//! Tests for `goto` and label resolution.
//!
//! Labels are function-scoped (C17 §6.2.1p3).  Every `goto L;` must have
//! a matching `L:` somewhere in the same function, duplicate labels are
//! an error, and unreferenced labels are legal.

use crate::stmt::analyze_function_def;
use crate::{context::SemaContext, scope::SymbolTable};

use super::helpers::*;

fn run(func: forge_parser::ast::FunctionDef) -> SemaContext {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    analyze_function_def(&func, &mut table, &ti(), &mut ctx);
    ctx
}

#[test]
fn goto_with_matching_label_accepts() {
    // void f(void) { start: goto start; }
    let body = vec![h_bstmt(h_label("start", h_goto("start")))];
    let ctx = run(h_fn_void_void("f", body));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn goto_undefined_label_is_error() {
    // void f(void) { goto nowhere; }
    let body = vec![h_bstmt(h_goto("nowhere"))];
    let ctx = run(h_fn_void_void("f", body));
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("use of undeclared label 'nowhere'")),
        "expected undeclared-label error, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn duplicate_label_is_error() {
    // void f(void) { L: ; L: ; }
    let body = vec![
        h_bstmt(h_label("L", h_empty_stmt())),
        h_bstmt(h_label("L", h_empty_stmt())),
    ];
    let ctx = run(h_fn_void_void("f", body));
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("redefinition of label 'L'")),
        "expected duplicate-label error, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn unreferenced_label_is_silent() {
    // `unused:` but no `goto unused;` — legal in C17.
    let body = vec![h_bstmt(h_label("unused", h_empty_stmt()))];
    let ctx = run(h_fn_void_void("f", body));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn forward_goto_is_resolved() {
    // void f(void) { goto end; end: ; }
    let body = vec![
        h_bstmt(h_goto("end")),
        h_bstmt(h_label("end", h_empty_stmt())),
    ];
    let ctx = run(h_fn_void_void("f", body));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}
