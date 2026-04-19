//! Value-level tests for `__builtin_offsetof`.
//!
//! These tests use `_Static_assert` as an integer-constant-expression
//! probe: if sema's ICX evaluator returns the correct offset, the
//! static assertion succeeds and the translation unit is clean; if the
//! offset is wrong (or the builtin is unevaluated), the static
//! assertion fires a diagnostic.

use super::helpers::{assert_source_clean, assert_source_has_errors};

#[test]
fn offsetof_first_member_is_zero() {
    assert_source_clean(
        r#"
            struct S { int a; int b; int c; };
            _Static_assert(__builtin_offsetof(struct S, a) == 0, "a@0");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn offsetof_second_member_is_four() {
    assert_source_clean(
        r#"
            struct S { int a; int b; int c; };
            _Static_assert(__builtin_offsetof(struct S, b) == 4, "b@4");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn offsetof_third_member_is_eight() {
    assert_source_clean(
        r#"
            struct S { int a; int b; int c; };
            _Static_assert(__builtin_offsetof(struct S, c) == 8, "c@8");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn offsetof_nested_anonymous_member() {
    assert_source_clean(
        r#"
            struct Outer { struct { int x; int y; } inner; };
            _Static_assert(__builtin_offsetof(struct Outer, inner.y) == 4, "inner.y@4");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn offsetof_array_subscript() {
    assert_source_clean(
        r#"
            struct A { int arr[10]; };
            _Static_assert(__builtin_offsetof(struct A, arr[3]) == 12, "arr[3]@12");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn offsetof_mixed_field_then_subscript() {
    assert_source_clean(
        r#"
            struct Both { int head; int rows[4]; };
            _Static_assert(__builtin_offsetof(struct Both, rows[2]) == 12, "rows[2]@12");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn offsetof_static_assert_in_icx_context() {
    assert_source_clean(
        r#"
            _Static_assert(__builtin_offsetof(struct { int a; int b; }, b) == 4, "ok");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn offsetof_on_non_struct_is_error() {
    assert_source_has_errors(
        r#"
            _Static_assert(__builtin_offsetof(int, foo) == 0, "unreachable");
        "#,
    );
}

#[test]
fn offsetof_unknown_member_is_error() {
    assert_source_has_errors(
        r#"
            struct S { int a; int b; };
            _Static_assert(__builtin_offsetof(struct S, nonexistent) == 0, "unreachable");
        "#,
    );
}
