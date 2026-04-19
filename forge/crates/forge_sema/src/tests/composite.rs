//! C17 §6.2.7 composite type tests.
//!
//! When two declarations provide partial information about the same
//! type, the "composite" type is the merge that keeps whichever
//! declaration was more specific.  Arrays prefer the fixed size and
//! functions prefer the prototype.

use super::helpers::*;
use crate::types::{composite_type, ArraySize, Type};

#[test]
fn fixed_beats_incomplete_array_size() {
    let c = ctx();
    let incomplete = q(array_of(q(int()), ArraySize::Incomplete));
    let fixed = q(array_of(q(int()), ArraySize::Fixed(10)));

    let composite = composite_type(&incomplete, &fixed, &c);
    match &composite.ty {
        Type::Array { size, .. } => assert_eq!(size, &ArraySize::Fixed(10)),
        other => panic!("expected array, got {other:?}"),
    }

    // Symmetric.
    let composite_rev = composite_type(&fixed, &incomplete, &c);
    match &composite_rev.ty {
        Type::Array { size, .. } => assert_eq!(size, &ArraySize::Fixed(10)),
        other => panic!("expected array, got {other:?}"),
    }
}

#[test]
fn prototype_beats_unprototyped_function() {
    let c = ctx();
    let proto = q(func(q(int()), vec![q(int())], false));
    let old = q(func_noproto(q(int())));

    let composite = composite_type(&old, &proto, &c);
    match &composite.ty {
        Type::Function { is_prototype, .. } => assert!(*is_prototype),
        other => panic!("expected function, got {other:?}"),
    }

    let composite_rev = composite_type(&proto, &old, &c);
    match &composite_rev.ty {
        Type::Function { is_prototype, .. } => assert!(*is_prototype),
        other => panic!("expected function, got {other:?}"),
    }
}

#[test]
fn composite_of_two_fixed_arrays_keeps_size() {
    let c = ctx();
    let a = q(array_of(q(int()), ArraySize::Fixed(10)));
    let b = q(array_of(q(int()), ArraySize::Fixed(10)));

    let composite = composite_type(&a, &b, &c);
    match &composite.ty {
        Type::Array { size, .. } => assert_eq!(size, &ArraySize::Fixed(10)),
        other => panic!("expected array, got {other:?}"),
    }
}

#[test]
fn composite_nested_arrays_merges_inner_size() {
    // `int a[][4]` + `int a[5][4]`  →  `int[5][4]`
    let c = ctx();
    let inner = ArraySize::Fixed(4);
    let outer_incomplete = q(array_of(
        q(array_of(q(int()), inner.clone())),
        ArraySize::Incomplete,
    ));
    let outer_fixed = q(array_of(
        q(array_of(q(int()), inner.clone())),
        ArraySize::Fixed(5),
    ));

    let composite = composite_type(&outer_incomplete, &outer_fixed, &c);
    match &composite.ty {
        Type::Array { size, element } => {
            assert_eq!(size, &ArraySize::Fixed(5));
            match &element.ty {
                Type::Array { size, .. } => assert_eq!(size, &inner),
                other => panic!("inner: expected array, got {other:?}"),
            }
        }
        other => panic!("expected array, got {other:?}"),
    }
}

#[test]
fn composite_of_unrelated_types_returns_lhs_clone() {
    // `are_compatible` screens the caller — but the function is lenient
    // about mismatches and just returns `a`.
    let c = ctx();
    let a = q(int());
    let b = q(long());
    assert_eq!(composite_type(&a, &b, &c), a);
}
