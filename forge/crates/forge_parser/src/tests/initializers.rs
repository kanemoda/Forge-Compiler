//! Tests for Prompt 3.4 — brace-enclosed initializer lists with
//! designators.

use crate::ast::*;

use super::helpers::{parse_decl, parse_decls};

/// The sole initializer of a single-init-declarator declaration.
fn sole_init(d: &Declaration) -> &Initializer {
    assert_eq!(d.init_declarators.len(), 1);
    d.init_declarators[0]
        .initializer
        .as_ref()
        .expect("expected initializer")
}

/// Unwrap `Initializer::List`.
fn as_list(init: &Initializer) -> &[DesignatedInit] {
    match init {
        Initializer::List { items, .. } => items,
        Initializer::Expr(_) => panic!("expected Initializer::List, got Expr"),
    }
}

/// Unwrap `Initializer::Expr`.
fn as_expr(init: &Initializer) -> &Expr {
    match init {
        Initializer::Expr(e) => e,
        Initializer::List { .. } => panic!("expected Initializer::Expr, got List"),
    }
}

fn int_lit(e: &Expr) -> u64 {
    match e {
        Expr::IntLiteral { value, .. } => *value,
        other => panic!("expected IntLiteral, got {other:?}"),
    }
}

// =========================================================================
// Flat and nested initializer lists
// =========================================================================

#[test]
fn array_flat_list() {
    // `int a[] = {1, 2, 3};`
    let d = parse_decl("int a[] = {1, 2, 3};");
    let items = as_list(sole_init(&d));
    assert_eq!(items.len(), 3);
    for (i, item) in items.iter().enumerate() {
        assert!(
            item.designators.is_empty(),
            "item {i} should have no designator"
        );
        assert_eq!(int_lit(as_expr(&item.initializer)), (i + 1) as u64);
    }
}

#[test]
fn nested_matrix_init() {
    // `int a[2][2] = { {1, 2}, {3, 4} };`
    let d = parse_decl("int a[2][2] = { {1, 2}, {3, 4} };");
    let outer = as_list(sole_init(&d));
    assert_eq!(outer.len(), 2);

    let inner0 = as_list(&outer[0].initializer);
    assert_eq!(inner0.len(), 2);
    assert_eq!(int_lit(as_expr(&inner0[0].initializer)), 1);
    assert_eq!(int_lit(as_expr(&inner0[1].initializer)), 2);

    let inner1 = as_list(&outer[1].initializer);
    assert_eq!(inner1.len(), 2);
    assert_eq!(int_lit(as_expr(&inner1[0].initializer)), 3);
    assert_eq!(int_lit(as_expr(&inner1[1].initializer)), 4);
}

// =========================================================================
// Designators
// =========================================================================

#[test]
fn field_designators() {
    // `struct Point p = { .x = 1, .y = 2 };`
    let decls = parse_decls("struct Point { int x; int y; }; struct Point p = { .x = 1, .y = 2 };");
    assert_eq!(decls.len(), 2);
    let items = as_list(sole_init(&decls[1]));
    assert_eq!(items.len(), 2);

    match items[0].designators.as_slice() {
        [Designator::Field(n)] => assert_eq!(n, "x"),
        other => panic!("expected [.x], got {other:?}"),
    }
    assert_eq!(int_lit(as_expr(&items[0].initializer)), 1);

    match items[1].designators.as_slice() {
        [Designator::Field(n)] => assert_eq!(n, "y"),
        other => panic!("expected [.y], got {other:?}"),
    }
    assert_eq!(int_lit(as_expr(&items[1].initializer)), 2);
}

#[test]
fn index_designators() {
    // `int a[10] = { [5] = 50, [9] = 90 };`
    let d = parse_decl("int a[10] = { [5] = 50, [9] = 90 };");
    let items = as_list(sole_init(&d));
    assert_eq!(items.len(), 2);

    match items[0].designators.as_slice() {
        [Designator::Index(e)] => assert_eq!(int_lit(e), 5),
        other => panic!("expected [5], got {other:?}"),
    }
    assert_eq!(int_lit(as_expr(&items[0].initializer)), 50);

    match items[1].designators.as_slice() {
        [Designator::Index(e)] => assert_eq!(int_lit(e), 9),
        other => panic!("expected [9], got {other:?}"),
    }
    assert_eq!(int_lit(as_expr(&items[1].initializer)), 90);
}

#[test]
fn nested_designated_with_nested_list() {
    // `struct { struct Point pos; } s = { .pos = { .x = 1, .y = 2 } };`
    let src = "struct Point { int x; int y; }; \
               struct { struct Point pos; } s = { .pos = { .x = 1, .y = 2 } };";
    let decls = parse_decls(src);
    let items = as_list(sole_init(&decls[1]));
    assert_eq!(items.len(), 1);

    match items[0].designators.as_slice() {
        [Designator::Field(n)] => assert_eq!(n, "pos"),
        other => panic!("expected [.pos], got {other:?}"),
    }

    let inner = as_list(&items[0].initializer);
    assert_eq!(inner.len(), 2);
    match inner[0].designators.as_slice() {
        [Designator::Field(n)] => assert_eq!(n, "x"),
        other => panic!("inner [0]: expected [.x], got {other:?}"),
    }
    assert_eq!(int_lit(as_expr(&inner[0].initializer)), 1);
    match inner[1].designators.as_slice() {
        [Designator::Field(n)] => assert_eq!(n, "y"),
        other => panic!("inner [1]: expected [.y], got {other:?}"),
    }
    assert_eq!(int_lit(as_expr(&inner[1].initializer)), 2);
}

// =========================================================================
// Edge cases: trailing comma, empty list
// =========================================================================

#[test]
fn trailing_comma_is_ok() {
    let decls =
        parse_decls("struct Point { int x; int y; }; struct Point p = { .x = 1, .y = 2, };");
    let items = as_list(sole_init(&decls[1]));
    assert_eq!(items.len(), 2);
}

#[test]
fn empty_initializer_list() {
    // `int a[] = {};` — accepted as extension.
    let d = parse_decl("int a[] = {};");
    let items = as_list(sole_init(&d));
    assert!(items.is_empty());
}
