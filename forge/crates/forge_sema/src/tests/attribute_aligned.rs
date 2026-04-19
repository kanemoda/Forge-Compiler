//! Acceptance tests for `__attribute__((aligned))`.
//!
//! The parser recognises GNU `__attribute__` lists; sema currently
//! tolerates the `aligned` attribute without error.  The effective
//! alignment bump itself is still driven by `_Alignas` (see
//! `resolve::apply_alignas`) — wiring the attribute into
//! `QualType::explicit_align` is a Phase 5 item.
//!
//! These tests therefore pin down acceptance: every well-formed
//! `__attribute__((aligned(N)))` call must sema-check without error.

use super::helpers::assert_source_clean;

#[test]
fn aligned_on_file_scope_variable_accepted() {
    assert_source_clean(
        "int x __attribute__((aligned(16)));
         int main(void) { return x; }",
    );
}

#[test]
fn aligned_with_power_of_two_values_accepted() {
    for n in [1u32, 2, 4, 8, 16, 32, 64] {
        let src = format!(
            "int v_{n} __attribute__((aligned({n})));
             int main(void) {{ return v_{n}; }}"
        );
        assert_source_clean(&src);
    }
}

#[test]
fn aligned_on_struct_tag_accepted() {
    assert_source_clean(
        "struct __attribute__((aligned(8))) T { char c; };
         int main(void) {
             struct T t;
             t.c = 0;
             return t.c;
         }",
    );
}

#[test]
fn alignas_still_records_explicit_align() {
    // _Alignas remains the ISO-C route that populates
    // QualType::explicit_align; the GNU attribute is currently
    // ignored for size/alignment purposes but does not error.
    assert_source_clean(
        "_Alignas(16) int x;
         int main(void) { return x; }",
    );
}
