//! Acceptance tests for the `__builtin_va_*` variadic-argument helpers.
//!
//! The `__builtin_va_list` typedef and the three helper functions
//! (`__builtin_va_start`, `__builtin_va_end`, `__builtin_va_copy`) are
//! pre-seeded in the symbol table by `seed_builtin_functions` —
//! every variadic function in system headers relies on them.

use super::helpers::assert_source_clean;

#[test]
fn va_list_declaration_accepted() {
    assert_source_clean(
        "int main(void) {
             __builtin_va_list ap;
             (void)ap;
             return 0;
         }",
    );
}

#[test]
fn va_start_and_va_end_accepted() {
    assert_source_clean(
        r#"
            int vprintf_like(const char *fmt, ...) {
                __builtin_va_list ap;
                __builtin_va_start(ap, fmt);
                __builtin_va_end(ap);
                return 0;
            }
            int main(void) { return vprintf_like("hi"); }
        "#,
    );
}

#[test]
fn va_copy_accepted() {
    assert_source_clean(
        r#"
            int f(int n, ...) {
                __builtin_va_list a, b;
                __builtin_va_start(a, n);
                __builtin_va_copy(b, a);
                __builtin_va_end(b);
                __builtin_va_end(a);
                return 0;
            }
            int main(void) { return f(1); }
        "#,
    );
}
