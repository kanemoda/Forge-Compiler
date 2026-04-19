//! Size / alignment tables on the x86-64 Linux LP64 ABI.
//!
//! These values are locked by the System V AMD64 ABI and baked into
//! every libc / compiler on the platform — changing any of them is a
//! de-facto breaking change, so they deserve plain-old regression
//! tests.

use super::helpers::*;
use crate::types::ArraySize;

#[test]
fn lp64_scalar_sizes() {
    let t = ti();
    let c = ctx();

    assert_eq!(t_bool().size_of(&t, &c), Some(1));
    assert_eq!(char_plain().size_of(&t, &c), Some(1));
    assert_eq!(char_signed().size_of(&t, &c), Some(1));
    assert_eq!(char_unsigned().size_of(&t, &c), Some(1));
    assert_eq!(short().size_of(&t, &c), Some(2));
    assert_eq!(ushort().size_of(&t, &c), Some(2));
    assert_eq!(int().size_of(&t, &c), Some(4));
    assert_eq!(uint().size_of(&t, &c), Some(4));
    assert_eq!(long().size_of(&t, &c), Some(8));
    assert_eq!(ulong().size_of(&t, &c), Some(8));
    assert_eq!(llong().size_of(&t, &c), Some(8));
    assert_eq!(ullong().size_of(&t, &c), Some(8));
    assert_eq!(t_float().size_of(&t, &c), Some(4));
    assert_eq!(t_double().size_of(&t, &c), Some(8));
    assert_eq!(long_double().size_of(&t, &c), Some(16));
}

#[test]
fn void_has_no_size() {
    assert_eq!(void().size_of(&ti(), &ctx()), None);
}

#[test]
fn pointer_is_eight_bytes_on_lp64() {
    let t = ti();
    let c = ctx();

    let p = ptr_to(q(int()));
    assert_eq!(p.size_of(&t, &c), Some(8));
    assert_eq!(p.align_of(&t, &c), Some(8));

    let p_p = ptr_to(q(ptr_to(q(char_plain()))));
    assert_eq!(p_p.size_of(&t, &c), Some(8));
}

#[test]
fn fixed_array_size_is_elem_times_count() {
    let t = ti();
    let c = ctx();

    let int_10 = array_of(q(int()), ArraySize::Fixed(10));
    assert_eq!(int_10.size_of(&t, &c), Some(40));
    assert_eq!(int_10.align_of(&t, &c), Some(4));

    let ll_3 = array_of(q(llong()), ArraySize::Fixed(3));
    assert_eq!(ll_3.size_of(&t, &c), Some(24));
    assert_eq!(ll_3.align_of(&t, &c), Some(8));
}

#[test]
fn incomplete_or_variable_array_has_no_size() {
    let t = ti();
    let c = ctx();

    let open = array_of(q(int()), ArraySize::Incomplete);
    assert_eq!(open.size_of(&t, &c), None);
    // Alignment of the incomplete array falls back to the element's.
    assert_eq!(open.align_of(&t, &c), Some(4));

    let vla = array_of(q(int()), ArraySize::Variable);
    assert_eq!(vla.size_of(&t, &c), None);
}

#[test]
fn function_type_has_no_size() {
    let t = ti();
    let c = ctx();

    let f = func(q(int()), vec![q(int())], false);
    assert_eq!(f.size_of(&t, &c), None);
    assert_eq!(f.align_of(&t, &c), None);
}

#[test]
fn struct_size_comes_from_context() {
    let t = ti();
    let mut c = ctx();
    let sid = register_struct(&mut c, 0, "Foo", 12, 4);

    let s_ty = crate::types::Type::Struct(sid);
    assert_eq!(s_ty.size_of(&t, &c), Some(12));
    assert_eq!(s_ty.align_of(&t, &c), Some(4));
}

#[test]
fn incomplete_struct_has_no_size() {
    let t = ti();
    let mut c = ctx();
    let sid = crate::types::StructTypeId(9);
    c.set_struct(
        sid,
        crate::types::StructLayout {
            tag: Some("Forward".into()),
            ..crate::types::StructLayout::default()
        },
    );

    let s_ty = crate::types::Type::Struct(sid);
    assert_eq!(s_ty.size_of(&t, &c), None);
    assert!(!c.is_struct_complete(sid));
}

#[test]
fn enum_falls_back_to_int_size() {
    let t = ti();
    let c = ctx();

    let eid = crate::types::EnumTypeId(0);
    let e_ty = crate::types::Type::Enum(eid);

    // No info registered — should fall back to `int` size / alignment.
    assert_eq!(e_ty.size_of(&t, &c), Some(t.int_size));
    assert_eq!(e_ty.align_of(&t, &c), Some(t.int_size));
}

#[test]
fn target_size_t_ptrdiff_t_wchar_t_on_lp64() {
    let t = ti();
    assert_eq!(t.size_t_type(), ulong());
    assert_eq!(t.ptrdiff_t_type(), long());
    assert_eq!(t.wchar_t_type(), int());
}
