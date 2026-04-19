//! C17 §6.3.1.1 integer promotion tests.
//!
//! A small integer operand is promoted to `int` (or `unsigned int` when
//! the value cannot fit in a signed `int`) before arithmetic.  `int`,
//! `long`, and wider types are left alone.

use super::helpers::*;
use crate::types::integer_promotion;

#[test]
fn bool_promotes_to_signed_int() {
    assert_eq!(integer_promotion(&t_bool(), &ti()), int());
}

#[test]
fn char_variants_promote_to_signed_int() {
    let t = ti();
    assert_eq!(integer_promotion(&char_plain(), &t), int());
    assert_eq!(integer_promotion(&char_signed(), &t), int());
    assert_eq!(integer_promotion(&char_unsigned(), &t), int());
}

#[test]
fn signed_short_promotes_to_signed_int() {
    assert_eq!(integer_promotion(&short(), &ti()), int());
}

#[test]
fn unsigned_short_fits_in_int_on_lp64() {
    // On LP64 sizeof(int) > sizeof(short), so unsigned short promotes to
    // *signed* int rather than unsigned int.
    assert_eq!(integer_promotion(&ushort(), &ti()), int());
}

#[test]
fn int_passes_through_unchanged() {
    let t = ti();
    assert_eq!(integer_promotion(&int(), &t), int());
    assert_eq!(integer_promotion(&uint(), &t), uint());
}

#[test]
fn long_passes_through_unchanged() {
    let t = ti();
    assert_eq!(integer_promotion(&long(), &t), long());
    assert_eq!(integer_promotion(&ulong(), &t), ulong());
    assert_eq!(integer_promotion(&llong(), &t), llong());
    assert_eq!(integer_promotion(&ullong(), &t), ullong());
}

#[test]
fn enum_promotes_to_signed_int() {
    use crate::types::{EnumTypeId, Type};
    let e = Type::Enum(EnumTypeId(0));
    assert_eq!(integer_promotion(&e, &ti()), int());
}

#[test]
fn floats_pass_through_unchanged() {
    let t = ti();
    assert_eq!(integer_promotion(&t_float(), &t), t_float());
    assert_eq!(integer_promotion(&t_double(), &t), t_double());
    assert_eq!(integer_promotion(&long_double(), &t), long_double());
}

#[test]
fn unsigned_short_promotes_to_unsigned_int_when_int_equals_short() {
    // Construct a hypothetical target where `int` and `short` are the
    // same width — unsigned short can then *not* fit in signed int and
    // must promote to unsigned int.
    use crate::types::TargetInfo;

    let mut t = TargetInfo::x86_64_linux();
    t.int_size = 2;
    t.short_size = 2;

    assert_eq!(integer_promotion(&ushort(), &t), uint());
}
