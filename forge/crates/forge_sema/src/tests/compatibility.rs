//! C17 §6.2.7 type compatibility tests.
//!
//! Two types are compatible if they match structurally *and* their
//! top-level qualifiers agree.  [`are_compatible_unqualified`] is the
//! qualifier-blind variant used by e.g. assignment rules that diff
//! qualifiers separately.

use super::helpers::*;
use crate::types::{are_compatible, are_compatible_unqualified, ArraySize, StructTypeId, Type};

// ---------- void ----------

#[test]
fn void_is_compatible_with_void() {
    let c = ctx();
    assert!(are_compatible(&q(void()), &q(void()), &c));
}

// ---------- arithmetic ----------

#[test]
fn int_is_compatible_with_int() {
    let c = ctx();
    assert!(are_compatible(&q(int()), &q(int()), &c));
}

#[test]
fn int_is_not_compatible_with_uint() {
    let c = ctx();
    assert!(!are_compatible(&q(int()), &q(uint()), &c));
}

#[test]
fn int_is_not_compatible_with_long_on_lp64() {
    // Even though both are 4-byte on some targets, they are distinct C
    // types — never compatible.
    let c = ctx();
    assert!(!are_compatible(&q(int()), &q(long()), &c));
}

// ---------- qualifiers ----------

#[test]
fn const_int_is_not_compatible_with_int_qualified() {
    let c = ctx();
    assert!(!are_compatible(&q(int()).with_const(), &q(int()), &c));
}

#[test]
fn const_int_is_compatible_with_int_unqualified() {
    let c = ctx();
    assert!(are_compatible_unqualified(
        &q(int()).with_const(),
        &q(int()),
        &c
    ));
}

// ---------- pointers ----------

#[test]
fn pointer_to_int_matches_pointer_to_int() {
    let c = ctx();
    let a = q(ptr_to(q(int())));
    let b = q(ptr_to(q(int())));
    assert!(are_compatible(&a, &b, &c));
}

#[test]
fn pointer_pointee_qualifiers_must_match() {
    let c = ctx();
    let a = q(ptr_to(q(int())));
    let b = q(ptr_to(q(int()).with_const()));
    assert!(!are_compatible(&a, &b, &c));
}

// ---------- arrays ----------

#[test]
fn fixed_array_matches_same_size() {
    let c = ctx();
    let a = q(array_of(q(int()), ArraySize::Fixed(10)));
    let b = q(array_of(q(int()), ArraySize::Fixed(10)));
    assert!(are_compatible(&a, &b, &c));
}

#[test]
fn fixed_array_matches_incomplete_array() {
    let c = ctx();
    let a = q(array_of(q(int()), ArraySize::Fixed(10)));
    let b = q(array_of(q(int()), ArraySize::Incomplete));
    assert!(are_compatible(&a, &b, &c));
    assert!(are_compatible(&b, &a, &c));
}

#[test]
fn different_fixed_sizes_are_not_compatible() {
    let c = ctx();
    let a = q(array_of(q(int()), ArraySize::Fixed(10)));
    let b = q(array_of(q(int()), ArraySize::Fixed(20)));
    assert!(!are_compatible(&a, &b, &c));
}

// ---------- functions ----------

#[test]
fn identical_prototyped_functions_are_compatible() {
    let c = ctx();
    let a = q(func(q(int()), vec![q(int())], false));
    let b = q(func(q(int()), vec![q(int())], false));
    assert!(are_compatible(&a, &b, &c));
}

#[test]
fn different_param_types_are_not_compatible() {
    let c = ctx();
    let a = q(func(q(int()), vec![q(int())], false));
    let b = q(func(q(int()), vec![q(char_plain())], false));
    assert!(!are_compatible(&a, &b, &c));
}

#[test]
fn different_arities_are_not_compatible() {
    let c = ctx();
    let a = q(func(q(int()), vec![q(int())], false));
    let b = q(func(q(int()), vec![q(int()), q(int())], false));
    assert!(!are_compatible(&a, &b, &c));
}

#[test]
fn prototyped_and_unprototyped_compatible_when_default_promoted() {
    // `int (int)` vs `int()` — compatible because the single parameter
    // is already in its default-promoted form.
    let c = ctx();
    let proto = q(func(q(int()), vec![q(int())], false));
    let old = q(func_noproto(q(int())));
    assert!(are_compatible(&proto, &old, &c));
    assert!(are_compatible(&old, &proto, &c));
}

#[test]
fn prototyped_incompatible_if_param_needs_promotion() {
    // Prototype takes `char` — not default-promoted, so the old-style
    // `int()` declaration is NOT compatible with it.
    let c = ctx();
    let proto = q(func(q(int()), vec![q(char_plain())], false));
    let old = q(func_noproto(q(int())));
    assert!(!are_compatible(&proto, &old, &c));
}

#[test]
fn unprototyped_compatible_with_unprototyped() {
    let c = ctx();
    let a = q(func_noproto(q(int())));
    let b = q(func_noproto(q(int())));
    assert!(are_compatible(&a, &b, &c));
}

// ---------- struct / union tag identity ----------

#[test]
fn same_struct_id_is_compatible() {
    let mut c = ctx();
    let sid = register_struct(&mut c, 0, "A", 4, 4);
    let a = q(Type::Struct(sid));
    let b = q(Type::Struct(sid));
    assert!(are_compatible(&a, &b, &c));
}

#[test]
fn different_struct_ids_are_not_compatible() {
    let mut c = ctx();
    let a_id = register_struct(&mut c, 0, "A", 4, 4);
    let _b_id = register_struct(&mut c, 1, "B", 4, 4);

    let a = q(Type::Struct(a_id));
    let b = q(Type::Struct(StructTypeId(1)));
    assert!(!are_compatible(&a, &b, &c));
}

// ---------- enum ↔ int ----------

#[test]
fn enum_is_compatible_with_signed_int() {
    use crate::types::EnumTypeId;
    let c = ctx();
    let e = q(Type::Enum(EnumTypeId(0)));
    assert!(are_compatible(&e, &q(int()), &c));
    assert!(are_compatible(&q(int()), &e, &c));
}
