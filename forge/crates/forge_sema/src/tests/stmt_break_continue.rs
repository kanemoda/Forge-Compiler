//! Tests for `break` / `continue` context rules.
//!
//! C17 §6.8.6:
//!
//! * `break` may appear in a loop or a `switch` body.
//! * `continue` may appear only in a loop — a surrounding `switch` does
//!   *not* satisfy the requirement.

use crate::scope::{ScopeKind, SymbolTable};
use crate::stmt::analyze_stmt;
use crate::{context::SemaContext, types::QualType};

use super::helpers::*;

fn drive(stmt: forge_parser::ast::Stmt) -> SemaContext {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table.push_scope(ScopeKind::Function);
    let mut fnc = fn_ctx(QualType::unqualified(int()));
    analyze_stmt(&stmt, &mut fnc, &mut table, &ti(), &mut ctx);
    ctx
}

// ---------------------------------------------------------------------
// break
// ---------------------------------------------------------------------

#[test]
fn break_in_while_accepts() {
    let ctx = drive(h_while(h_int_lit(1), h_break()));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn break_in_for_accepts() {
    let ctx = drive(h_for(None, None, None, h_break()));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn break_in_do_while_accepts() {
    let ctx = drive(h_do_while(h_break(), h_int_lit(1)));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn break_in_switch_accepts() {
    let body = h_compound_stmt(vec![h_bstmt(h_case(h_int_lit(1), h_break()))]);
    let ctx = drive(h_switch(h_int_lit(0), body));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn break_outside_loop_or_switch_is_error() {
    let ctx = drive(h_break());
    assert!(
        ctx.diagnostics.iter().any(|d| d
            .message
            .contains("'break' statement not in loop or switch")),
        "expected break-outside error, got {:?}",
        ctx.diagnostics
    );
}

// ---------------------------------------------------------------------
// continue
// ---------------------------------------------------------------------

#[test]
fn continue_in_while_accepts() {
    let ctx = drive(h_while(h_int_lit(1), h_continue()));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn continue_in_for_accepts() {
    let ctx = drive(h_for(None, None, None, h_continue()));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn continue_outside_loop_is_error() {
    let ctx = drive(h_continue());
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("'continue' statement not in loop")),
        "expected continue-outside error, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn continue_in_switch_alone_is_error() {
    // C17 §6.8.6.2 — a `switch` alone does not satisfy `continue`.
    let body = h_compound_stmt(vec![h_bstmt(h_case(h_int_lit(1), h_continue()))]);
    let ctx = drive(h_switch(h_int_lit(0), body));
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("'continue' statement not in loop")),
        "expected continue-in-switch-without-loop error, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn continue_in_switch_inside_loop_accepts() {
    // `while (1) switch (0) { case 1: continue; }` is legal — the `continue`
    // binds to the enclosing `while`.
    let switch_body = h_compound_stmt(vec![h_bstmt(h_case(h_int_lit(1), h_continue()))]);
    let switch_stmt = h_switch(h_int_lit(0), switch_body);
    let ctx = drive(h_while(h_int_lit(1), switch_stmt));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}
