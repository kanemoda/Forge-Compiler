//! Tests for `<<`, `>>`, `&&`, `||`, and the bitwise `&` / `|` / `^`
//! operators (Prompt 4.5).
//!
//! Shifts promote each operand independently; the result type is the
//! promoted *left* operand.  A literal RHS `>=` the LHS bit width must
//! emit a warning.  Bitwise operators require integer operands and
//! apply the usual arithmetic conversions.  Logical operators accept
//! any scalar operand and always yield `int`.

use forge_diagnostics::Severity;
use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::Expr;
use forge_parser::ast_ops::BinaryOp;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{QualType, Type};

use super::helpers::{int, ptr_to, q, short, t_double, t_float, ti};

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
fn shl_returns_promoted_left_type() {
    // short << int — after integer promotion of the left operand the
    // result should be int (NOT the usual arithmetic conversion common
    // type, which would also be int here but would be wrong for a
    // `long << int` case).
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "s", q(short()), &mut ctx);
    let e = bin(BinaryOp::Shl, ident("s", 1), int_lit(1, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
}

#[test]
fn shl_long_by_int_returns_long() {
    // long << int should stay long — proving the result is the promoted
    // LEFT, not the usual-arithmetic-conversion common type.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(
        &mut table,
        "l",
        q(Type::Long { is_unsigned: false }),
        &mut ctx,
    );
    let e = bin(BinaryOp::Shl, ident("l", 1), int_lit(1, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Long { is_unsigned: false });
}

#[test]
fn shr_on_float_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "f", q(t_float()), &mut ctx);
    let e = bin(BinaryOp::Shr, ident("f", 1), int_lit(1, 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("integer"));
}

#[test]
fn shl_with_constant_count_too_large_warns() {
    // 1 << 64 on int — LHS promoted is 32-bit, so 64 is out of range.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = bin(BinaryOp::Shl, int_lit(1, 1), int_lit(64, 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "must be warning only");
    assert!(ctx
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Warning && d.message.contains("shift count")));
}

#[test]
fn bitand_two_ints_yields_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = bin(BinaryOp::BitAnd, int_lit(0xff, 1), int_lit(0x0f, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
}

#[test]
fn bitor_rejects_float() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "f", q(t_float()), &mut ctx);
    let e = bin(BinaryOp::BitOr, ident("f", 1), int_lit(1, 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
}

#[test]
fn bitxor_applies_usual_arithmetic_conversions() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(
        &mut table,
        "u",
        q(Type::Long { is_unsigned: true }),
        &mut ctx,
    );
    let e = bin(BinaryOp::BitXor, ident("u", 1), int_lit(1, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Long { is_unsigned: true });
}

#[test]
fn logand_scalar_operands_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    // ptr && int — both scalar, result is int.
    let e = bin(BinaryOp::LogAnd, ident("p", 1), int_lit(1, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, int());
}

#[test]
fn logor_rejects_struct_operand() {
    use crate::types::StructLayout;
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let sid = ctx.type_ctx.fresh_struct_id();
    ctx.type_ctx.set_struct(
        sid,
        StructLayout {
            is_complete: true,
            total_size: 4,
            alignment: 4,
            ..StructLayout::default()
        },
    );
    declare_var(&mut table, "s", q(Type::Struct(sid)), &mut ctx);
    let e = bin(BinaryOp::LogOr, ident("s", 1), int_lit(1, 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("scalar"));
}

#[test]
fn logical_and_of_two_doubles_yields_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "a", q(t_double()), &mut ctx);
    declare_var(&mut table, "b", q(t_double()), &mut ctx);
    let e = bin(BinaryOp::LogAnd, ident("a", 1), ident("b", 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
}
