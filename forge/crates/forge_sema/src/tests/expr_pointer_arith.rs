//! Tests for pointer arithmetic (Prompt 4.5).
//!
//! `ptr + int`, `int + ptr`, `ptr - int` return the pointer type;
//! `ptr - ptr` returns `ptrdiff_t`.  `void *` arithmetic is a GCC-style
//! warning, not an error.  Arithmetic between two pointers or between a
//! pointer and a non-integer is rejected.

use forge_diagnostics::Severity;
use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::Expr;
use forge_parser::ast_ops::BinaryOp;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{QualType, Type};

use super::helpers::{int, ptr_to, q, t_double, ti, void};

const S: Span = Span::primary(0, 0);

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
fn ptr_plus_int_returns_pointer() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    let e = bin(BinaryOp::Add, ident("p", 1), int_lit(3, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, ptr_to(q(int())));
}

#[test]
fn int_plus_ptr_returns_pointer() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    let e = bin(BinaryOp::Add, int_lit(3, 1), ident("p", 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, ptr_to(q(int())));
}

#[test]
fn ptr_minus_int_returns_pointer() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    let e = bin(BinaryOp::Sub, ident("p", 1), int_lit(3, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, ptr_to(q(int())));
}

#[test]
fn ptr_minus_ptr_returns_ptrdiff_t() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    declare_var(&mut table, "q", q(ptr_to(q(int()))), &mut ctx);
    let e = bin(BinaryOp::Sub, ident("p", 1), ident("q", 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    // ptrdiff_t on LP64 is signed long.
    assert_eq!(qt.ty, Type::Long { is_unsigned: false });
}

#[test]
fn ptr_plus_ptr_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    declare_var(&mut table, "q", q(ptr_to(q(int()))), &mut ctx);
    let e = bin(BinaryOp::Add, ident("p", 1), ident("q", 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("add two pointers"));
}

#[test]
fn void_ptr_plus_int_is_warning_not_error() {
    // `void *p + 1` — GCC accepts this with a warning.  We must not
    // stop compilation, but we must emit a diagnostic.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(void()))), &mut ctx);
    let e = bin(BinaryOp::Add, ident("p", 1), int_lit(1, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "must be warning only");
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Warning),
        "expected a warning for void* arithmetic"
    );
    assert_eq!(qt.ty, ptr_to(q(void())));
}

#[test]
fn ptr_minus_incompatible_ptr_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    declare_var(&mut table, "q", q(ptr_to(q(t_double()))), &mut ctx);
    let e = bin(BinaryOp::Sub, ident("p", 1), ident("q", 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("compatible"));
}

#[test]
fn ptr_minus_float_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    declare_var(&mut table, "d", q(t_double()), &mut ctx);
    let e = bin(BinaryOp::Sub, ident("p", 1), ident("d", 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
}
