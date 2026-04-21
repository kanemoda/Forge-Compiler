//! Tests for `arr[i]` and the reversed `i[arr]` form (Prompt 4.4).

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::Expr;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{ArraySize, ImplicitConversion, QualType, Type};

use super::helpers::{int, q, ti};

const S: Span = Span::primary(0, 0);

fn ident(name: &str, id: u32) -> Expr {
    Expr::Ident {
        name: name.to_string(),
        span: S,
        node_id: NodeId(id),
    }
}

fn int_lit(value: u64, id: u32) -> Expr {
    Expr::IntLiteral {
        value,
        suffix: IntSuffix::None,
        span: S,
        node_id: NodeId(id),
    }
}

fn subscript(array: Expr, index: Expr, id: u32) -> Expr {
    Expr::ArraySubscript {
        array: Box::new(array),
        index: Box::new(index),
        span: S,
        node_id: NodeId(id),
    }
}

fn declare_var(table: &mut SymbolTable, name: &str, ty: QualType, ctx: &mut SemaContext) {
    let sym = Symbol {
        id: 0,
        name: name.to_string(),
        ty,
        kind: SymbolKind::Variable,
        storage: StorageClass::None,
        linkage: Linkage::None,
        span: S,
        is_defined: true,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
    };
    table.declare(sym, ctx).expect("declare must succeed");
}

fn int_array_10() -> QualType {
    q(Type::Array {
        element: Box::new(q(int())),
        size: ArraySize::Fixed(10),
    })
}

#[test]
fn subscript_on_array_decays_and_yields_element_lvalue() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "arr", int_array_10(), &mut ctx);

    let e = subscript(ident("arr", 1), int_lit(3, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    assert_eq!(qt.ty, int());
    assert!(ctx.is_lvalue(NodeId(3)), "arr[i] is an lvalue");

    // The array operand should have been decayed to a pointer.
    assert_eq!(
        ctx.implicit_convs.get(&1),
        Some(&ImplicitConversion::ArrayToPointer)
    );
}

#[test]
fn subscript_reversed_form_is_equivalent() {
    // `3[arr]` is lexically swapped but semantically identical to `arr[3]`.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "arr", int_array_10(), &mut ctx);

    let e = subscript(int_lit(3, 10), ident("arr", 11), 12);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    assert_eq!(qt.ty, int());
    assert!(ctx.is_lvalue(NodeId(12)));
}

#[test]
fn subscript_two_integers_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "n", q(int()), &mut ctx);

    let e = subscript(ident("n", 20), int_lit(0, 21), 22);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0]
        .message
        .contains("pointer and an integer"));
}

#[test]
fn subscript_of_pointer_variable_works() {
    // int *p;  p[0];
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(
        &mut table,
        "p",
        q(Type::Pointer {
            pointee: Box::new(q(int())),
        }),
        &mut ctx,
    );

    let e = subscript(ident("p", 30), int_lit(0, 31), 32);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    assert_eq!(qt.ty, int());
    assert!(ctx.is_lvalue(NodeId(32)));
}
