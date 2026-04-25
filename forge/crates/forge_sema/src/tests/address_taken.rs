//! Tests for the address-taken analysis pass (Phase 5 prerequisite).
//!
//! Each test compiles a small C fragment end-to-end (lex + parse +
//! sema) and inspects the resulting [`SymbolTable`] for the
//! `address_taken` flag on a specific local.  The pass should mark a
//! local whenever `&local` or array-to-pointer decay applies to it
//! anywhere in scope, and should leave parameters and globals at the
//! `false` default (Phase 5 treats those as memory-resident regardless).

use forge_diagnostics::Severity;

use crate::scope::{Symbol, SymbolKind, SymbolTable};

use super::helpers::analyze_source;

// ---------------------------------------------------------------------
// Local helpers
// ---------------------------------------------------------------------

fn assert_no_errors(diags: &[forge_diagnostics::Diagnostic], src: &str) {
    let errors: Vec<&forge_diagnostics::Diagnostic> = diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Error))
        .collect();
    assert!(
        errors.is_empty(),
        "expected no errors, got: {errors:?}\n\nfull source:\n{src}"
    );
}

fn find_variable<'a>(table: &'a SymbolTable, name: &str) -> &'a Symbol {
    table
        .all_symbols()
        .iter()
        .find(|s| s.name == name && matches!(s.kind, SymbolKind::Variable))
        .unwrap_or_else(|| panic!("variable '{name}' not found in symbol table"))
}

fn find_parameter<'a>(table: &'a SymbolTable, name: &str) -> &'a Symbol {
    table
        .all_symbols()
        .iter()
        .find(|s| s.name == name && matches!(s.kind, SymbolKind::Parameter))
        .unwrap_or_else(|| panic!("parameter '{name}' not found in symbol table"))
}

// ---------------------------------------------------------------------
// Negative cases — locals that are NOT address-taken
// ---------------------------------------------------------------------

#[test]
fn scalar_local_used_in_arithmetic_is_not_address_taken() {
    let src = r#"
        void f(void) {
            int x = 5;
            int y = x + 1;
            (void)y;
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let x = find_variable(&table, "x");
    assert!(
        !x.address_taken,
        "x is only read for its value; address_taken should be false"
    );
}

#[test]
fn local_used_only_as_sizeof_operand_is_not_address_taken() {
    // sizeof's operand is unevaluated (C17 §6.5.3.4) — the walker
    // deliberately skips recursion into it, so even an array name there
    // does not count as an escape.
    let src = r#"
        typedef unsigned long size_t;
        void f(void) {
            int x = 5;
            size_t s = sizeof x;
            (void)s;
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let x = find_variable(&table, "x");
    assert!(
        !x.address_taken,
        "sizeof does not evaluate its operand; address_taken should be false"
    );
}

// ---------------------------------------------------------------------
// Positive cases — direct `&local`
// ---------------------------------------------------------------------

#[test]
fn scalar_local_addressed_with_ampersand_is_address_taken() {
    let src = r#"
        void f(void) {
            int x = 5;
            int *p = &x;
            (void)p;
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let x = find_variable(&table, "x");
    assert!(x.address_taken, "&x must mark x as address_taken");
}

#[test]
fn scalar_local_passed_to_pointer_param_is_address_taken() {
    let src = r#"
        void g(int *);
        void f(void) {
            int x = 5;
            g(&x);
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let x = find_variable(&table, "x");
    assert!(x.address_taken, "g(&x) must mark x as address_taken");
}

#[test]
fn scalar_local_passed_to_anonymous_pointer_param_is_address_taken() {
    // Regression guard for the Phase 4 fix-up: anonymous pointer
    // parameters used to lose their `*` in the parser, which made sema
    // reject `g(&x)` as "incompatible types" before the address-taken
    // pass had a chance to mark x.
    let src = r#"
        void g(int *);
        void f(void) {
            int x = 5;
            g(&x);
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let x = find_variable(&table, "x");
    assert!(
        x.address_taken,
        "g(&x) through an anonymous pointer parameter must still mark x"
    );
}

#[test]
fn scalar_local_assigned_to_pointer_is_address_taken() {
    let src = r#"
        void f(void) {
            int x = 5;
            int *p;
            p = &x;
            (void)p;
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let x = find_variable(&table, "x");
    assert!(x.address_taken, "p = &x must mark x as address_taken");
}

#[test]
fn scalar_local_address_stored_into_struct_field_is_address_taken() {
    let src = r#"
        struct S { int *p; };
        void f(void) {
            int x = 5;
            struct S s;
            s.p = &x;
            (void)s;
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let x = find_variable(&table, "x");
    assert!(x.address_taken, "s.p = &x must mark x as address_taken");
}

#[test]
fn scalar_local_address_stored_via_double_indirection_is_address_taken() {
    let src = r#"
        void f(void) {
            int x = 5;
            int *p;
            int **pp = &p;
            *pp = &x;
            (void)x;
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let x = find_variable(&table, "x");
    assert!(x.address_taken, "*pp = &x must mark x as address_taken");
}

#[test]
fn scalar_local_address_stored_into_global_is_address_taken() {
    let src = r#"
        int *g_ptr;
        void f(void) {
            int x = 5;
            g_ptr = &x;
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let x = find_variable(&table, "x");
    assert!(x.address_taken, "g_ptr = &x must mark x as address_taken");
}

#[test]
fn scalar_local_returned_as_pointer_is_address_taken() {
    // Returning &x is undefined behaviour at runtime, but sema must
    // still flag x as address-taken so Phase 5 puts it in memory.
    let src = r#"
        int *f(void) {
            int x = 5;
            return &x;
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let x = find_variable(&table, "x");
    assert!(x.address_taken, "return &x must mark x as address_taken");
}

// ---------------------------------------------------------------------
// Positive cases — array-to-pointer decay
// ---------------------------------------------------------------------

#[test]
fn array_local_decayed_to_pointer_is_address_taken() {
    let src = r#"
        void g(int *);
        void f(void) {
            int arr[10];
            g(arr);
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let arr = find_variable(&table, "arr");
    assert!(
        arr.address_taken,
        "ArrayToPointer decay on local arr must mark it as address_taken"
    );
}

// ---------------------------------------------------------------------
// Defensive: parameters and globals are not tracked but must not crash
// ---------------------------------------------------------------------

#[test]
fn parameter_address_taken_does_not_crash() {
    let src = r#"
        void f(int x) {
            int *p = &x;
            (void)p;
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let x = find_parameter(&table, "x");
    // Phase 4 deliberately leaves parameters at the default false; Phase
    // 5 will treat them as memory-resident unconditionally.
    assert!(
        !x.address_taken,
        "parameter address_taken must stay at the default false"
    );
}

#[test]
fn global_address_taken_does_not_crash() {
    let src = r#"
        int g;
        void f(void) {
            int *p = &g;
            (void)p;
        }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    assert_no_errors(&diags, src);
    let g = find_variable(&table, "g");
    // File-scope variables have linkage other than None, so the analysis
    // skips them — Phase 5 treats globals as memory-resident anyway.
    assert!(
        !g.address_taken,
        "global address_taken must stay at the default false"
    );
}
