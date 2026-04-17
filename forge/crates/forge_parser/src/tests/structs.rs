//! Tests for Prompt 3.4 — struct and union definitions.

use crate::ast::*;

use super::helpers::parse_decl;

/// Extract the struct definition from a declaration whose sole type
/// specifier is a `struct` (or `union`).
fn struct_def(d: &Declaration) -> &StructDef {
    assert_eq!(
        d.specifiers.type_specifiers.len(),
        1,
        "expected one type specifier, got {:?}",
        d.specifiers.type_specifiers
    );
    match &d.specifiers.type_specifiers[0] {
        TypeSpecifierToken::Struct(s) | TypeSpecifierToken::Union(s) => s,
        other => panic!("expected Struct/Union, got {other:?}"),
    }
}

/// Unwrap a `StructMember::Field` or panic.
fn field(m: &StructMember) -> &StructField {
    match m {
        StructMember::Field(f) => f,
        other => panic!("expected Field member, got {other:?}"),
    }
}

fn field_name(f: &StructFieldDeclarator) -> Option<&str> {
    let d = f.declarator.as_ref()?;
    crate::decl::declarator_name(d)
}

// =========================================================================
// Basic struct/union definitions
// =========================================================================

#[test]
fn struct_point_two_fields() {
    let d = parse_decl("struct Point { int x; int y; };");
    let s = struct_def(&d);
    assert_eq!(s.kind, StructOrUnion::Struct);
    assert_eq!(s.name.as_deref(), Some("Point"));
    let members = s.members.as_ref().expect("expected member list");
    assert_eq!(members.len(), 2);

    let f0 = field(&members[0]);
    assert_eq!(f0.declarators.len(), 1);
    assert_eq!(field_name(&f0.declarators[0]), Some("x"));
    assert!(f0.declarators[0].bit_width.is_none());

    let f1 = field(&members[1]);
    assert_eq!(field_name(&f1.declarators[0]), Some("y"));
}

#[test]
fn anonymous_struct_with_declarator() {
    // `struct { int x; } anon;` — no tag, body present.
    let d = parse_decl("struct { int x; } anon;");
    let s = struct_def(&d);
    assert!(s.name.is_none());
    let members = s.members.as_ref().expect("body");
    assert_eq!(members.len(), 1);
    assert_eq!(field_name(&field(&members[0]).declarators[0]), Some("x"));

    // Outer init-declarator is `anon`.
    assert_eq!(d.init_declarators.len(), 1);
    assert_eq!(
        crate::decl::declarator_name(&d.init_declarators[0].declarator),
        Some("anon")
    );
}

#[test]
fn struct_forward_declaration() {
    let d = parse_decl("struct S;");
    let s = struct_def(&d);
    assert_eq!(s.name.as_deref(), Some("S"));
    assert!(s.members.is_none(), "forward ref must have members=None");
    assert!(d.init_declarators.is_empty());
}

// =========================================================================
// Bit-fields
// =========================================================================

#[test]
fn bit_fields_named() {
    // `struct Flags { unsigned a : 1; unsigned b : 3; };`
    let d = parse_decl("struct Flags { unsigned a : 1; unsigned b : 3; };");
    let s = struct_def(&d);
    let members = s.members.as_ref().expect("body");
    assert_eq!(members.len(), 2);

    let f0 = field(&members[0]);
    let d0 = &f0.declarators[0];
    assert_eq!(field_name(d0), Some("a"));
    match d0.bit_width.as_deref() {
        Some(Expr::IntLiteral { value, .. }) => assert_eq!(*value, 1),
        other => panic!("expected bit_width=1, got {other:?}"),
    }

    let f1 = field(&members[1]);
    let d1 = &f1.declarators[0];
    assert_eq!(field_name(d1), Some("b"));
    match d1.bit_width.as_deref() {
        Some(Expr::IntLiteral { value, .. }) => assert_eq!(*value, 3),
        other => panic!("expected bit_width=3, got {other:?}"),
    }
}

#[test]
fn anonymous_bit_field_plus_named() {
    // `struct { int : 4; int x : 4; };`
    let d = parse_decl("struct { int : 4; int x : 4; };");
    let s = struct_def(&d);
    let members = s.members.as_ref().expect("body");
    assert_eq!(members.len(), 2);

    let f0 = field(&members[0]);
    assert_eq!(f0.declarators.len(), 1);
    assert!(
        f0.declarators[0].declarator.is_none(),
        "anonymous bit-field must have declarator=None"
    );
    match f0.declarators[0].bit_width.as_deref() {
        Some(Expr::IntLiteral { value, .. }) => assert_eq!(*value, 4),
        other => panic!("expected bit_width=4, got {other:?}"),
    }

    let f1 = field(&members[1]);
    assert_eq!(field_name(&f1.declarators[0]), Some("x"));
    match f1.declarators[0].bit_width.as_deref() {
        Some(Expr::IntLiteral { value, .. }) => assert_eq!(*value, 4),
        other => panic!("expected bit_width=4, got {other:?}"),
    }
}

// =========================================================================
// Self-referential / nested / flexible / union / static-assert
// =========================================================================

#[test]
fn self_referential_via_pointer() {
    // `struct Node { int val; struct Node *next; };`
    let d = parse_decl("struct Node { int val; struct Node *next; };");
    let s = struct_def(&d);
    let members = s.members.as_ref().expect("body");
    assert_eq!(members.len(), 2);

    // Second member's specifiers are `struct Node` (forward ref).
    let f1 = field(&members[1]);
    assert_eq!(f1.specifiers.type_specifiers.len(), 1);
    match &f1.specifiers.type_specifiers[0] {
        TypeSpecifierToken::Struct(inner) => {
            assert_eq!(inner.name.as_deref(), Some("Node"));
            assert!(inner.members.is_none(), "inner should be forward ref");
        }
        other => panic!("expected Struct specifier, got {other:?}"),
    }

    let d1 = &f1.declarators[0];
    let declarator = d1.declarator.as_ref().expect("concrete declarator");
    assert_eq!(declarator.pointers.len(), 1);
    assert_eq!(crate::decl::declarator_name(declarator), Some("next"));
}

#[test]
fn nested_struct_definition() {
    // `struct Outer { struct Inner { int x; } inner; int y; };`
    let d = parse_decl("struct Outer { struct Inner { int x; } inner; int y; };");
    let s = struct_def(&d);
    let members = s.members.as_ref().expect("outer body");
    assert_eq!(members.len(), 2);

    // First member: specifier is a full struct Inner { int x; } definition.
    let f0 = field(&members[0]);
    match &f0.specifiers.type_specifiers[0] {
        TypeSpecifierToken::Struct(inner) => {
            assert_eq!(inner.name.as_deref(), Some("Inner"));
            let inner_members = inner.members.as_ref().expect("inner body");
            assert_eq!(inner_members.len(), 1);
            assert_eq!(
                field_name(&field(&inner_members[0]).declarators[0]),
                Some("x")
            );
        }
        other => panic!("expected Struct, got {other:?}"),
    }
    assert_eq!(field_name(&f0.declarators[0]), Some("inner"));

    // Second member: int y.
    let f1 = field(&members[1]);
    assert_eq!(field_name(&f1.declarators[0]), Some("y"));
}

#[test]
fn union_three_members() {
    let d = parse_decl("union Val { int i; float f; double d; };");
    let u = struct_def(&d);
    assert_eq!(u.kind, StructOrUnion::Union);
    let members = u.members.as_ref().expect("body");
    assert_eq!(members.len(), 3);
    assert_eq!(field_name(&field(&members[0]).declarators[0]), Some("i"));
    assert_eq!(field_name(&field(&members[1]).declarators[0]), Some("f"));
    assert_eq!(field_name(&field(&members[2]).declarators[0]), Some("d"));
}

#[test]
fn anonymous_union_member_c11() {
    // `struct S { union { int x; float f; }; int y; };`
    // Inner union has no declarator at all — the field has zero
    // struct-declarators.
    let d = parse_decl("struct S { union { int x; float f; }; int y; };");
    let s = struct_def(&d);
    let members = s.members.as_ref().expect("body");
    assert_eq!(members.len(), 2);

    let f0 = field(&members[0]);
    assert!(
        f0.declarators.is_empty(),
        "anonymous union member must have zero declarators, got {:?}",
        f0.declarators
    );
    match &f0.specifiers.type_specifiers[0] {
        TypeSpecifierToken::Union(inner) => {
            let inner_members = inner.members.as_ref().expect("union body");
            assert_eq!(inner_members.len(), 2);
        }
        other => panic!("expected Union specifier, got {other:?}"),
    }

    let f1 = field(&members[1]);
    assert_eq!(field_name(&f1.declarators[0]), Some("y"));
}

#[test]
fn flexible_array_member() {
    // `struct S { int n; int data[]; };`
    let d = parse_decl("struct S { int n; int data[]; };");
    let s = struct_def(&d);
    let members = s.members.as_ref().expect("body");
    assert_eq!(members.len(), 2);

    let f1 = field(&members[1]);
    let d1 = &f1.declarators[0];
    let decl = d1.declarator.as_ref().expect("concrete declarator");
    let DirectDeclarator::Array { size, .. } = &decl.direct else {
        panic!("expected Array declarator, got {:?}", decl.direct);
    };
    assert!(matches!(size, ArraySize::Unspecified));
}

#[test]
fn static_assert_in_struct_body() {
    // `struct S { _Static_assert(sizeof(int) == 4, "oops"); int x; };`
    let d = parse_decl("struct S { _Static_assert(sizeof(int) == 4, \"oops\"); int x; };");
    let s = struct_def(&d);
    let members = s.members.as_ref().expect("body");
    assert_eq!(members.len(), 2);

    match &members[0] {
        StructMember::StaticAssert(sa) => {
            assert_eq!(sa.message.as_deref(), Some("oops"));
        }
        other => panic!("expected StaticAssert, got {other:?}"),
    }

    match &members[1] {
        StructMember::Field(f) => {
            assert_eq!(field_name(&f.declarators[0]), Some("x"));
        }
        other => panic!("expected Field, got {other:?}"),
    }
}
