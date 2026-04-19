//! Acceptance tests for the GNU `__typeof__` operator.
//!
//! `__typeof__(expr)` and `__typeof__(type-name)` introduce the type of
//! their argument as a fresh type specifier, including qualifiers.  The
//! expression form is **unevaluated** — it yields a type without any
//! runtime side effects.

use super::helpers::{analyze_source, assert_source_clean};
use forge_diagnostics::Severity;

#[test]
fn typeof_of_int_literal_is_int() {
    assert_source_clean("__typeof__(42) x; int main(void) { x = 7; return x; }");
}

#[test]
fn typeof_of_pointer_type_is_pointer() {
    assert_source_clean(
        "__typeof__(int *) p;
         int main(void) { p = 0; return 0; }",
    );
}

#[test]
fn typeof_preserves_const_qualifier() {
    // A `const int` variable passed through __typeof__ gives back a
    // `const int` type; a second declaration using that type must still
    // accept an initialiser at its definition site.
    assert_source_clean(
        "const int x = 1;
         __typeof__(x) y = 2;
         int main(void) { return y; }",
    );
}

#[test]
fn typeof_expression_is_unevaluated() {
    // `f()` inside __typeof__ is not actually called — if it were, sema
    // would demand it be a compile-time constant in the file scope
    // initializer below.  Instead only its *type* is taken.
    let src = r#"
        int f(void);
        __typeof__(f()) z;
        int main(void) { z = 0; return z; }
    "#;
    // No errors — the call-site is never evaluated, so no "function
    // call is not a constant expression" diagnostic fires.
    let (diags, _ctx, _table) = analyze_source(src);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Error))
        .collect();
    assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
}
