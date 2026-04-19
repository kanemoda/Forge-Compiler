//! Tests for `__builtin_constant_p`.
//!
//! Unlike the other two type-taking builtins, `__builtin_constant_p`
//! reaches sema as a regular function call (the parser does not
//! intercept it).  Sema special-cases the callee in `check_call` and
//! `const_eval::eval` so it both type-checks and constant-folds: the
//! result is `1` when the argument is a compile-time constant,
//! otherwise `0`.

use super::helpers::assert_source_clean;

#[test]
fn integer_literal_is_constant() {
    assert_source_clean(
        r#"
            _Static_assert(__builtin_constant_p(42) == 1,
                           "42 must be recognised as constant");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn arithmetic_of_constants_is_constant() {
    assert_source_clean(
        r#"
            _Static_assert(__builtin_constant_p(2 + 3) == 1,
                           "2 + 3 is a compile-time constant");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn sizeof_of_type_is_constant() {
    assert_source_clean(
        r#"
            _Static_assert(__builtin_constant_p(sizeof(int)) == 1,
                           "sizeof(int) is a compile-time constant");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn automatic_variable_is_not_constant() {
    assert_source_clean(
        r#"
            int main(void) {
                int x = 0;
                int probe = __builtin_constant_p(x);
                return probe;
            }
            _Static_assert(1, "compile-only marker");
        "#,
    );
    // An automatic variable is not an integer constant expression, so
    // the probe returns 0.  We verify that at top scope via an enum
    // constant (which is evaluated as an ICX).
    assert_source_clean(
        r#"
            int g;
            enum { PROBE = __builtin_constant_p(g) };
            _Static_assert(PROBE == 0, "identifier is not constant");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn function_call_argument_is_not_constant() {
    assert_source_clean(
        r#"
            int foo(void);
            enum { PROBE = __builtin_constant_p(foo()) };
            _Static_assert(PROBE == 0, "function call is not constant");
            int main(void) { return 0; }
        "#,
    );
}
