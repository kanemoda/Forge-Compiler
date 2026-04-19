//! Acceptance tests for `noreturn` on function declarations.
//!
//! Two sources of the same semantic bit:
//!
//! * C11 `_Noreturn` — a function specifier.  Sema records it in
//!   `Symbol::is_noreturn` (populated from
//!   `FunctionSpecifier::Noreturn` in the parser).
//! * GNU `__attribute__((noreturn))` — a type attribute.  Phase 4
//!   currently accepts the attribute syntactically but does **not**
//!   yet map it onto `Symbol::has_noreturn_attr`; wiring the side is
//!   a Phase 5 item.
//!
//! These tests pin down what Phase 4 *does* cover: the attribute
//! form must sema-check, and `_Noreturn` must flip
//! `Symbol::is_noreturn`.

use super::helpers::analyze_source;
use forge_diagnostics::Severity;

#[test]
fn attribute_noreturn_on_prototype_accepted() {
    let src = r#"
        void die(void) __attribute__((noreturn));
        int main(void) { (void)die; return 0; }
    "#;
    let (diags, _ctx, _table) = analyze_source(src);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Error))
        .collect();
    assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
}

#[test]
fn c11_noreturn_sets_is_noreturn() {
    let src = r#"
        _Noreturn void die2(void);
        int main(void) { (void)die2; return 0; }
    "#;
    let (diags, _ctx, table) = analyze_source(src);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Error))
        .collect();
    assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    let sym = table.lookup("die2").expect("die2 must be declared");
    assert!(sym.is_noreturn, "_Noreturn must set Symbol::is_noreturn");
}

#[test]
fn calling_noreturn_function_does_not_warn_about_missing_return() {
    // A function that always calls a noreturn function with no fall-off
    // return still type-checks.  Our reachability analysis is lenient,
    // so this currently does not error regardless; the test exists to
    // pin the behaviour and flag a regression if that ever changes.
    let src = r#"
        _Noreturn void die(void);
        int f(void) { die(); }
        int main(void) { (void)f; return 0; }
    "#;
    let (diags, _ctx, _table) = analyze_source(src);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Error))
        .collect();
    assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
}
