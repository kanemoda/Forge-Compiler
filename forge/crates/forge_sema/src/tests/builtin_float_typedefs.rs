//! Acceptance tests for the `_FloatN` and `_FloatNx` typedefs.
//!
//! ISO/IEC TS 18661-3 introduced fixed-width floating types.  Until
//! Forge gains native `Type::Float16` / `Type::Float128` variants,
//! `tu::seed_builtin_typedef_names` seeds them as typedefs on the
//! closest native type:
//!
//! | typedef     | sema approximation |
//! |-------------|--------------------|
//! | `_Float16`  | `float`            |
//! | `_Float32`  | `float`            |
//! | `_Float32x` | `double`           |
//! | `_Float64`  | `double`           |
//! | `_Float64x` | `long double`      |
//! | `_Float128` | `long double`      |
//! | `_Float128x`| `long double`      |
//!
//! The sizes and precisions are wrong for the 16-bit and 128-bit
//! entries.  Native variants land in Phase 5.

use super::helpers::assert_source_clean;

#[test]
fn float32_declaration_accepted() {
    assert_source_clean(
        "int main(void) {
             _Float32 f32 = 1;
             return (int)f32;
         }",
    );
}

#[test]
fn float64_declaration_accepted() {
    assert_source_clean(
        "int main(void) {
             _Float64 f64 = 2;
             return (int)f64;
         }",
    );
}

#[test]
fn float128_declaration_accepted() {
    assert_source_clean(
        "int main(void) {
             _Float128 f128 = 3;
             return (int)f128;
         }",
    );
}

#[test]
fn float16_declaration_accepted() {
    // _Float16 approximates float — the declaration must at least
    // sema-check; TODO(phase5): verify size_of once a real type exists.
    assert_source_clean(
        "int main(void) {
             _Float16 f16 = 1;
             return (int)f16;
         }",
    );
}

#[test]
fn floatn_typedefs_interop_in_arithmetic() {
    // Until the approximation is replaced, arithmetic between _FloatN
    // values obeys the usual arithmetic conversions of their native
    // approximants.
    assert_source_clean(
        "int main(void) {
             _Float32 a = 1;
             _Float64 b = 2;
             _Float128 c = 3;
             return (int)(a + b + c);
         }",
    );
}
