//! Tests for `switch` / `case` / `default` checking.
//!
//! Covers the integer-controlling-expression rule, duplicate `case`
//! detection, duplicate `default` detection, and the requirement that
//! `case` and `default` live inside a `switch` body.

use crate::scope::{ScopeKind, SymbolTable};
use crate::stmt::analyze_stmt;
use crate::{context::SemaContext, types::QualType};

use super::helpers::*;

fn new_table() -> (SemaContext, SymbolTable) {
    let mut table = SymbolTable::new();
    table.push_scope(ScopeKind::Function);
    (SemaContext::new(), table)
}

// ---------------------------------------------------------------------
// Controlling expression
// ---------------------------------------------------------------------

#[test]
fn switch_on_integer_accepts() {
    let (mut ctx, mut table) = new_table();
    let mut fnc = fn_ctx(QualType::unqualified(int()));
    let body = h_compound_stmt(vec![h_bstmt(h_case(h_int_lit(1), h_break()))]);
    let s = h_switch(h_int_lit(1), body);
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

// ---------------------------------------------------------------------
// case / default outside of switch
// ---------------------------------------------------------------------

#[test]
fn case_outside_switch_is_error() {
    let (mut ctx, mut table) = new_table();
    let mut fnc = fn_ctx(QualType::unqualified(int()));
    let s = h_case(h_int_lit(1), h_empty_stmt());
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("'case' label is not inside a switch")),
        "expected case-outside-switch error, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn default_outside_switch_is_error() {
    let (mut ctx, mut table) = new_table();
    let mut fnc = fn_ctx(QualType::unqualified(int()));
    let s = h_default(h_empty_stmt());
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("'default' label is not inside a switch")),
        "expected default-outside-switch error, got {:?}",
        ctx.diagnostics
    );
}

// ---------------------------------------------------------------------
// Duplicate case values
// ---------------------------------------------------------------------

#[test]
fn duplicate_case_value_is_error() {
    let (mut ctx, mut table) = new_table();
    let mut fnc = fn_ctx(QualType::unqualified(int()));
    let body = h_compound_stmt(vec![
        h_bstmt(h_case(h_int_lit(1), h_break())),
        h_bstmt(h_case(h_int_lit(1), h_break())),
    ]);
    let s = h_switch(h_int_lit(0), body);
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("duplicate case value")),
        "expected duplicate-case error, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn distinct_case_values_accept() {
    let (mut ctx, mut table) = new_table();
    let mut fnc = fn_ctx(QualType::unqualified(int()));
    let body = h_compound_stmt(vec![
        h_bstmt(h_case(h_int_lit(1), h_break())),
        h_bstmt(h_case(h_int_lit(2), h_break())),
        h_bstmt(h_case(h_int_lit(3), h_break())),
    ]);
    let s = h_switch(h_int_lit(0), body);
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

// ---------------------------------------------------------------------
// Duplicate default
// ---------------------------------------------------------------------

#[test]
fn multiple_default_is_error() {
    let (mut ctx, mut table) = new_table();
    let mut fnc = fn_ctx(QualType::unqualified(int()));
    let body = h_compound_stmt(vec![
        h_bstmt(h_default(h_break())),
        h_bstmt(h_default(h_break())),
    ]);
    let s = h_switch(h_int_lit(0), body);
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("multiple default labels")),
        "expected multiple-default error, got {:?}",
        ctx.diagnostics
    );
}

// ---------------------------------------------------------------------
// Nested switches keep separate case sets
// ---------------------------------------------------------------------

#[test]
fn nested_switch_can_reuse_case_value() {
    let (mut ctx, mut table) = new_table();
    let mut fnc = fn_ctx(QualType::unqualified(int()));
    let inner = h_switch(
        h_int_lit(0),
        h_compound_stmt(vec![h_bstmt(h_case(h_int_lit(1), h_break()))]),
    );
    let outer_body = h_compound_stmt(vec![
        h_bstmt(h_case(h_int_lit(1), h_break())),
        h_bstmt(inner),
    ]);
    let s = h_switch(h_int_lit(0), outer_body);
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}
