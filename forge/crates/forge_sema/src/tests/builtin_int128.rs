//! Acceptance tests for the `__int128` family of types.
//!
//! # Phase 5 / codegen note
//!
//! Forge's v1 type system does not have a real 128-bit integer — the
//! three GCC names (`__int128`, `__int128_t`, `__uint128_t`) are seeded
//! as **typedefs** for `long long` / `unsigned long long` (see
//! `tu::seed_builtin_typedef_names`).  That is wrong for size and
//! alignment, and the `long long` approximation is known to diverge
//! from GCC on 128-bit arithmetic.  Real support lands when we grow
//! a dedicated `Type::Int128 { is_unsigned }` variant — tracked in
//! Phase 5.
//!
//! Until then these tests document the typedef approximation so the
//! Phase 5 promoter sees the intent and updates the expectations
//! accordingly.

use super::helpers::assert_source_clean;

#[test]
fn int128_declaration_accepted() {
    // TODO(phase5): switch to a real Int128 type and a size_of check.
    assert_source_clean(
        "int main(void) {
             __int128 big = 0;
             return (int)big;
         }",
    );
}

#[test]
fn int128_t_alias_accepted() {
    assert_source_clean(
        "int main(void) {
             __int128_t s = 0;
             return (int)s;
         }",
    );
}

#[test]
fn uint128_t_alias_accepted() {
    assert_source_clean(
        "int main(void) {
             __uint128_t u = 0;
             return (int)u;
         }",
    );
}

#[test]
fn int128_interops_with_fundamental_integers() {
    // While the approximation lasts, `__int128 + int` resolves as a
    // `long long` arithmetic operation — no surprises at sema level.
    assert_source_clean(
        "int main(void) {
             __int128 a = 1;
             int b = 2;
             return (int)(a + b);
         }",
    );
}
