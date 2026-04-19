//! Value-level tests for `__builtin_types_compatible_p`.
//!
//! These tests use `_Static_assert` to probe the ICX value returned
//! by the builtin.  Compatibility is evaluated ignoring top-level
//! qualifiers per the C17 semantics assumed by GCC's extension, so
//! e.g. `const int` is compatible with `int`.

use super::helpers::assert_source_clean;

#[test]
fn same_scalar_types_compatible() {
    assert_source_clean(
        r#"
            _Static_assert(__builtin_types_compatible_p(int, int) == 1, "int == int");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn distinct_integer_types_are_incompatible() {
    assert_source_clean(
        r#"
            _Static_assert(__builtin_types_compatible_p(int, long) == 0, "int != long");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn top_level_qualifier_is_ignored() {
    assert_source_clean(
        r#"
            _Static_assert(__builtin_types_compatible_p(const int, int) == 1,
                           "const int ~ int (qualifier-agnostic)");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn identical_pointer_types_compatible() {
    assert_source_clean(
        r#"
            _Static_assert(__builtin_types_compatible_p(int *, int *) == 1, "int* == int*");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn pointer_to_distinct_types_incompatible() {
    assert_source_clean(
        r#"
            _Static_assert(__builtin_types_compatible_p(int *, long *) == 0, "int* != long*");
            int main(void) { return 0; }
        "#,
    );
}

#[test]
fn types_compatible_p_in_icx_context() {
    assert_source_clean(
        r#"
            _Static_assert(__builtin_types_compatible_p(int, int), "ok");
            int main(void) { return 0; }
        "#,
    );
}
