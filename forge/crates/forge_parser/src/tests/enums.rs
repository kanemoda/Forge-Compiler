//! Tests for Prompt 3.4 — enum definitions.

use crate::ast::*;

use super::helpers::parse_decl;

fn enum_def(d: &Declaration) -> &EnumDef {
    assert_eq!(d.specifiers.type_specifiers.len(), 1);
    match &d.specifiers.type_specifiers[0] {
        TypeSpecifierToken::Enum(e) => e,
        other => panic!("expected Enum, got {other:?}"),
    }
}

#[test]
fn enum_color_no_values() {
    let d = parse_decl("enum Color { RED, GREEN, BLUE };");
    let e = enum_def(&d);
    assert_eq!(e.name.as_deref(), Some("Color"));
    let list = e.enumerators.as_ref().expect("enumerator list");
    assert_eq!(list.len(), 3);
    let names: Vec<&str> = list.iter().map(|en| en.name.as_str()).collect();
    assert_eq!(names, vec!["RED", "GREEN", "BLUE"]);
    for en in list {
        assert!(en.value.is_none(), "no explicit value expected");
    }
}

#[test]
fn enum_with_explicit_values() {
    let d = parse_decl("enum { A = 0, B = 5, C };");
    let e = enum_def(&d);
    assert!(e.name.is_none());
    let list = e.enumerators.as_ref().expect("list");
    assert_eq!(list.len(), 3);

    assert_eq!(list[0].name, "A");
    match list[0].value.as_deref() {
        Some(Expr::IntLiteral { value, .. }) => assert_eq!(*value, 0),
        other => panic!("expected A=0, got {other:?}"),
    }

    assert_eq!(list[1].name, "B");
    match list[1].value.as_deref() {
        Some(Expr::IntLiteral { value, .. }) => assert_eq!(*value, 5),
        other => panic!("expected B=5, got {other:?}"),
    }

    assert_eq!(list[2].name, "C");
    assert!(list[2].value.is_none(), "C has no explicit value");
}

#[test]
fn enum_trailing_comma_allowed() {
    let d = parse_decl("enum E { X, Y, Z, };");
    let e = enum_def(&d);
    let list = e.enumerators.as_ref().expect("list");
    assert_eq!(list.len(), 3);
    let names: Vec<&str> = list.iter().map(|en| en.name.as_str()).collect();
    assert_eq!(names, vec!["X", "Y", "Z"]);
}

#[test]
fn enum_forward_reference() {
    let d = parse_decl("enum E;");
    let e = enum_def(&d);
    assert_eq!(e.name.as_deref(), Some("E"));
    assert!(e.enumerators.is_none());
}
