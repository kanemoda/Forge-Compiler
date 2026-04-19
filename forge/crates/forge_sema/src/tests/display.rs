//! [`QualType`] pretty-printing.
//!
//! Diagnostics rely on `QualType: Display` producing readable,
//! C-ish syntax.  The declarator-style shape for pointer-to-function
//! and pointer-to-array is the tricky bit, so we cover it explicitly.

use super::helpers::*;
use crate::types::{ArraySize, QualType};

// ---------- base scalar types ----------

#[test]
fn base_scalar_names() {
    assert_eq!(q(void()).to_string(), "void");
    assert_eq!(q(t_bool()).to_string(), "_Bool");
    assert_eq!(q(char_plain()).to_string(), "char");
    assert_eq!(q(char_signed()).to_string(), "signed char");
    assert_eq!(q(char_unsigned()).to_string(), "unsigned char");
    assert_eq!(q(short()).to_string(), "short");
    assert_eq!(q(ushort()).to_string(), "unsigned short");
    assert_eq!(q(int()).to_string(), "int");
    assert_eq!(q(uint()).to_string(), "unsigned int");
    assert_eq!(q(long()).to_string(), "long");
    assert_eq!(q(ulong()).to_string(), "unsigned long");
    assert_eq!(q(llong()).to_string(), "long long");
    assert_eq!(q(ullong()).to_string(), "unsigned long long");
    assert_eq!(q(t_float()).to_string(), "float");
    assert_eq!(q(t_double()).to_string(), "double");
    assert_eq!(q(long_double()).to_string(), "long double");
}

// ---------- qualifiers on scalars ----------

#[test]
fn const_int_renders_with_leading_const() {
    assert_eq!(q(int()).with_const().to_string(), "const int");
}

#[test]
fn volatile_int_renders_with_leading_volatile() {
    let mut qt = q(int());
    qt.is_volatile = true;
    assert_eq!(qt.to_string(), "volatile int");
}

// ---------- pointers and pointer qualifiers ----------

#[test]
fn pointer_to_int_has_one_space() {
    let qt = q(ptr_to(q(int())));
    assert_eq!(qt.to_string(), "int *");
}

#[test]
fn pointer_to_const_int() {
    let qt = q(ptr_to(q(int()).with_const()));
    assert_eq!(qt.to_string(), "const int *");
}

#[test]
fn const_pointer_to_int_renders_const_after_star() {
    let qt = q(ptr_to(q(int()))).with_const();
    assert_eq!(qt.to_string(), "int *const");
}

#[test]
fn pointer_to_pointer_to_int() {
    // The printer emits a leading space before each star.
    let qt = q(ptr_to(q(ptr_to(q(int())))));
    assert_eq!(qt.to_string(), "int * *");
}

// ---------- arrays ----------

#[test]
fn array_of_int() {
    let qt = q(array_of(q(int()), ArraySize::Fixed(10)));
    assert_eq!(qt.to_string(), "int[10]");
}

#[test]
fn incomplete_array_renders_empty_brackets() {
    let qt = q(array_of(q(int()), ArraySize::Incomplete));
    assert_eq!(qt.to_string(), "int[]");
}

// ---------- function and function-pointer types ----------

#[test]
fn function_type_renders_return_and_params() {
    let qt = q(func(q(int()), vec![q(int()), q(int())], false));
    assert_eq!(qt.to_string(), "int(int, int)");
}

#[test]
fn function_pointer_uses_declarator_syntax() {
    let inner = q(func(q(int()), vec![q(int()), q(int())], false));
    let qt = q(ptr_to(inner));
    assert_eq!(qt.to_string(), "int (*)(int, int)");
}

#[test]
fn pointer_to_fixed_array_uses_declarator_syntax() {
    let inner = q(array_of(q(int()), ArraySize::Fixed(5)));
    let qt = q(ptr_to(inner));
    assert_eq!(qt.to_string(), "int (*)[5]");
}

// ---------- struct / union / enum tags ----------

#[test]
fn struct_renders_tag_via_context() {
    let mut c = ctx();
    let sid = register_struct(&mut c, 0, "Foo", 4, 4);

    let qt = QualType::unqualified(crate::types::Type::Struct(sid));
    assert_eq!(qt.to_c_string(&c), "struct Foo");
}

#[test]
fn struct_without_context_falls_back_to_hash_id() {
    let qt = QualType::unqualified(crate::types::Type::Struct(crate::types::StructTypeId(3)));
    // Display has no context; the printer uses `struct #<id>`.
    assert_eq!(qt.to_string(), "struct #3");
}

#[test]
fn union_renders_tag_via_context() {
    let mut c = ctx();
    let uid = register_union(&mut c, 7, "Bar", 8, 8);

    let qt = QualType::unqualified(crate::types::Type::Union(uid));
    assert_eq!(qt.to_c_string(&c), "union Bar");
}
