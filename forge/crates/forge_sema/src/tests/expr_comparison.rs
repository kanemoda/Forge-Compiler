//! Tests for equality and relational operators (Prompt 4.5).
//!
//! Comparisons yield `int` in every case.  Arithmetic operands apply
//! the usual arithmetic conversions; pointer operands require
//! compatibility (or a `void *` or null pointer constant).  Ordered
//! comparisons involving `void *` emit a warning but still succeed.

use forge_diagnostics::Severity;
use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::Expr;
use forge_parser::ast_ops::BinaryOp;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::QualType;

use super::helpers::{int, ptr_to, q, t_double, ti, void};

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
fn equality_of_two_ints_yields_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = bin(BinaryOp::Eq, int_lit(1, 1), int_lit(2, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
}

#[test]
fn equality_of_pointer_and_null_constant_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    let e = bin(BinaryOp::Ne, ident("p", 1), int_lit(0, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, int());
}

#[test]
fn equality_of_two_compatible_pointers_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    declare_var(&mut table, "q", q(ptr_to(q(int()))), &mut ctx);
    let e = bin(BinaryOp::Eq, ident("p", 1), ident("q", 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
}

#[test]
fn equality_of_ptr_and_double_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    declare_var(&mut table, "d", q(t_double()), &mut ctx);
    let e = bin(BinaryOp::Eq, ident("p", 1), ident("d", 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
}

#[test]
fn less_than_two_doubles_yields_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "a", q(t_double()), &mut ctx);
    declare_var(&mut table, "b", q(t_double()), &mut ctx);
    let e = bin(BinaryOp::Lt, ident("a", 1), ident("b", 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
}

#[test]
fn relational_void_pointer_is_warning() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(void()))), &mut ctx);
    declare_var(&mut table, "q", q(ptr_to(q(int()))), &mut ctx);
    let e = bin(BinaryOp::Lt, ident("p", 1), ident("q", 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "must be a warning, not an error");
    assert!(ctx
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Warning));
    assert_eq!(qt.ty, int());
}

#[test]
fn relational_incompatible_pointers_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    declare_var(&mut table, "q", q(ptr_to(q(t_double()))), &mut ctx);
    let e = bin(BinaryOp::Lt, ident("p", 1), ident("q", 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("incompatible"));
}

#[test]
fn greater_than_int_and_double_promotes_to_double_then_int_result() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "i", q(int()), &mut ctx);
    declare_var(&mut table, "d", q(t_double()), &mut ctx);
    let e = bin(BinaryOp::Gt, ident("i", 1), ident("d", 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
    // The LHS must have been converted to double before the compare.
    assert!(ctx.implicit_convs.contains_key(&1));
}

#[test]
fn equal_to_pointer_and_nonzero_int_literal_is_error() {
    // A non-zero int literal is NOT a null pointer constant, so this
    // mixed comparison is a type error.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    let e = bin(BinaryOp::Eq, ident("p", 1), int_lit(7, 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
}

#[test]
fn le_and_ge_on_ints_yields_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let le = bin(BinaryOp::Le, int_lit(1, 1), int_lit(2, 2), 3);
    let ge = bin(BinaryOp::Ge, int_lit(1, 4), int_lit(2, 5), 6);
    let qt1 = check_expr(&le, &mut table, &ti(), &mut ctx);
    let qt2 = check_expr(&ge, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt1.ty, int());
    assert_eq!(qt2.ty, int());
}
