//! Tests for pre- and postfix `++` / `--` (Prompt 4.5).
//!
//! Both forms share the same set of operand requirements: a modifiable
//! (i.e. non-`const`) lvalue whose type is arithmetic or pointer-to-
//! complete.  The result is NOT an lvalue.

use forge_lexer::Span;
use forge_parser::ast::Expr;
use forge_parser::ast_ops::{PostfixOp, UnaryOp};
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

fn int_lit(id: u32) -> Expr {
    Expr::IntLiteral {
        value: 1,
        suffix: forge_lexer::IntSuffix::None,
        span: S,
        node_id: NodeId(id),
    }
}

fn pre_inc(operand: Expr, id: u32) -> Expr {
    Expr::UnaryOp {
        op: UnaryOp::PreIncrement,
        operand: Box::new(operand),
        span: S,
        node_id: NodeId(id),
    }
}

fn post_inc(operand: Expr, id: u32) -> Expr {
    Expr::PostfixOp {
        op: PostfixOp::PostIncrement,
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
fn pre_increment_int_variable_ok_and_not_lvalue() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "x", q(int()), &mut ctx);
    let e = pre_inc(ident("x", 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
    assert!(!ctx.is_lvalue(NodeId(2)));
}

#[test]
fn post_increment_rvalue_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = post_inc(int_lit(1), 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("lvalue"));
}

#[test]
fn pre_increment_const_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let mut qt = q(int());
    qt.is_const = true;
    declare_var(&mut table, "c", qt, &mut ctx);
    let e = pre_inc(ident("c", 1), 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("const"));
}

#[test]
fn pre_increment_pointer_to_int_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    let e = pre_inc(ident("p", 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, ptr_to(q(int())));
}

#[test]
fn pre_increment_pointer_to_void_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(void()))), &mut ctx);
    let e = pre_inc(ident("p", 1), 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("incomplete"));
}

#[test]
fn post_increment_double_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "d", q(t_double()), &mut ctx);
    let e = post_inc(ident("d", 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Double);
}

#[test]
fn pre_increment_struct_is_error() {
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
    let e = pre_inc(ident("s", 1), 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(
        ctx.diagnostics[0].message.contains("arithmetic")
            || ctx.diagnostics[0].message.contains("pointer")
    );
}
