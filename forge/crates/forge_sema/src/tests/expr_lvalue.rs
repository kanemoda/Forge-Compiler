//! Tests for lvalue marking and default conversions (Prompt 4.4).
//!
//! These tests verify that `check_expr` marks the right nodes as lvalues
//! and records the correct implicit conversions (ArrayToPointer,
//! LvalueToRvalue) in [`SemaContext::implicit_convs`].

use forge_lexer::{IntSuffix, Span, StringPrefix};
use forge_parser::ast::Expr;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::{check_expr, check_expr_in_context, ValueContext};
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{ArraySize, ImplicitConversion, QualType, Type};

use super::helpers::{int, q, ti};

const S: Span = Span::new(0, 0);

fn ident(name: &str, id: u32) -> Expr {
    Expr::Ident {
        name: name.to_string(),
        span: S,
        node_id: NodeId(id),
    }
}

fn int_lit(v: u64, id: u32) -> Expr {
    Expr::IntLiteral {
        value: v,
        suffix: IntSuffix::None,
        span: S,
        node_id: NodeId(id),
    }
}

fn string_lit(value: &str, id: u32) -> Expr {
    Expr::StringLiteral {
        value: value.to_string(),
        prefix: StringPrefix::None,
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

#[test]
fn variable_reference_is_marked_as_lvalue() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "x", q(int()), &mut ctx);

    let _ = check_expr(&ident("x", 1), &mut table, &ti(), &mut ctx);
    assert!(ctx.is_lvalue(NodeId(1)));
}

#[test]
fn int_literal_is_not_an_lvalue() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let _ = check_expr(&int_lit(42, 2), &mut table, &ti(), &mut ctx);
    assert!(!ctx.is_lvalue(NodeId(2)));
}

#[test]
fn string_literal_is_marked_as_lvalue() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let _ = check_expr(&string_lit("hello", 3), &mut table, &ti(), &mut ctx);
    assert!(ctx.is_lvalue(NodeId(3)));
}

#[test]
fn rvalue_context_records_lvalue_to_rvalue_conversion() {
    // Reading `x` in an rvalue context applies L2R.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "x", q(int()), &mut ctx);

    let _ = check_expr(&ident("x", 10), &mut table, &ti(), &mut ctx);
    assert_eq!(
        ctx.implicit_convs.get(&10),
        Some(&ImplicitConversion::LvalueToRvalue)
    );
}

#[test]
fn sizeof_context_suppresses_lvalue_to_rvalue() {
    // `check_expr_in_context(x, SizeofOperand, ...)` should NOT record a
    // LvalueToRvalue conversion because the value is never loaded.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "x", q(int()), &mut ctx);

    let _ = check_expr_in_context(
        &ident("x", 11),
        ValueContext::SizeofOperand,
        &mut table,
        &ti(),
        &mut ctx,
    );
    assert_eq!(ctx.implicit_convs.get(&11), None);
}

#[test]
fn array_identifier_decays_to_pointer_in_rvalue_context() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(
        &mut table,
        "arr",
        q(Type::Array {
            element: Box::new(q(int())),
            size: ArraySize::Fixed(5),
        }),
        &mut ctx,
    );

    let qt = check_expr(&ident("arr", 20), &mut table, &ti(), &mut ctx);
    assert_eq!(
        ctx.implicit_convs.get(&20),
        Some(&ImplicitConversion::ArrayToPointer)
    );
    match qt.ty {
        Type::Pointer { pointee } => assert_eq!(pointee.ty, int()),
        other => panic!("expected int*, got {other:?}"),
    }
}

#[test]
fn sizeof_context_suppresses_array_decay() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(
        &mut table,
        "arr",
        q(Type::Array {
            element: Box::new(q(int())),
            size: ArraySize::Fixed(5),
        }),
        &mut ctx,
    );

    let qt = check_expr_in_context(
        &ident("arr", 21),
        ValueContext::SizeofOperand,
        &mut table,
        &ti(),
        &mut ctx,
    );
    assert_eq!(ctx.implicit_convs.get(&21), None);
    // Array should not have decayed.
    assert!(matches!(qt.ty, Type::Array { .. }));
}
