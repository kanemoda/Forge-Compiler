//! Tests for arithmetic binary operators and unary prefix operators
//! (Prompt 4.5).
//!
//! Every test here would fail if the relevant operator handler were
//! removed or regressed: binary arithmetic must apply the usual
//! arithmetic conversions, `%` must reject floats, unary `+`/`-`/`~`
//! must trigger integer promotion, and unary `!` must yield `int`
//! regardless of operand.

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::Expr;
use forge_parser::ast_ops::{BinaryOp, UnaryOp};
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{ImplicitConversion, QualType, Type};

use super::helpers::{int, q, short, t_double, t_float, ti};

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

fn bin(op: BinaryOp, l: Expr, r: Expr, id: u32) -> Expr {
    Expr::BinaryOp {
        op,
        left: Box::new(l),
        right: Box::new(r),
        span: S,
        node_id: NodeId(id),
    }
}

fn un(op: UnaryOp, operand: Expr, id: u32) -> Expr {
    Expr::UnaryOp {
        op,
        operand: Box::new(operand),
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
fn add_two_ints_yields_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = bin(BinaryOp::Add, int_lit(1, 1), int_lit(2, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, int());
}

#[test]
fn add_int_and_double_promotes_to_double() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "i", q(int()), &mut ctx);
    declare_var(&mut table, "d", q(t_double()), &mut ctx);
    let e = bin(BinaryOp::Add, ident("i", 1), ident("d", 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Double);
    // The int operand must be converted up to double.
    assert_eq!(
        ctx.implicit_convs.get(&1),
        Some(&ImplicitConversion::IntToFloat { to: Type::Double })
    );
}

#[test]
fn sub_two_floats_yields_float() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "a", q(t_float()), &mut ctx);
    declare_var(&mut table, "b", q(t_float()), &mut ctx);
    let e = bin(BinaryOp::Sub, ident("a", 1), ident("b", 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Float);
}

#[test]
fn mul_applies_usual_arithmetic_conversions() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "s", q(short()), &mut ctx);
    let e = bin(BinaryOp::Mul, ident("s", 1), int_lit(3, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    // short * int → int (via integer promotion on the short operand).
    assert_eq!(qt.ty, int());
}

#[test]
fn div_rejects_pointer_operand() {
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
    let e = bin(BinaryOp::Div, ident("p", 1), int_lit(1, 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("arithmetic"));
}

#[test]
fn mod_rejects_float_operand() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "d", q(t_double()), &mut ctx);
    let e = bin(BinaryOp::Mod, ident("d", 1), int_lit(2, 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("integer"));
}

#[test]
fn unary_plus_on_short_promotes_to_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "s", q(short()), &mut ctx);
    let e = un(UnaryOp::Plus, ident("s", 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
    assert_eq!(
        ctx.implicit_convs.get(&1),
        Some(&ImplicitConversion::IntegerPromotion { to: int() })
    );
}

#[test]
fn unary_minus_on_double_stays_double() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "d", q(t_double()), &mut ctx);
    let e = un(UnaryOp::Minus, ident("d", 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Double);
}

#[test]
fn unary_plus_rejects_pointer() {
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
    let e = un(UnaryOp::Plus, ident("p", 1), 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
}

#[test]
fn unary_bitnot_rejects_float() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "f", q(t_float()), &mut ctx);
    let e = un(UnaryOp::BitNot, ident("f", 1), 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("integer"));
}

#[test]
fn unary_bitnot_on_short_promotes_to_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "s", q(short()), &mut ctx);
    let e = un(UnaryOp::BitNot, ident("s", 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
}

#[test]
fn unary_lognot_on_int_yields_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = un(UnaryOp::LogNot, int_lit(5, 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
}

#[test]
fn unary_lognot_on_pointer_yields_int() {
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
    let e = un(UnaryOp::LogNot, ident("p", 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
}
