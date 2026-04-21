//! Tests for `&expr` and `*expr` (Prompt 4.5).
//!
//! Address-of suppresses the default conversions on its operand,
//! requires an lvalue (or a function designator), and yields a pointer
//! whose pointee carries the operand's qualifiers.  Dereference
//! extracts the pointee type; the result is an lvalue *unless* the
//! pointee is a function type, in which case the expression is a
//! function designator and the outer default conversion will re-apply
//! function-to-pointer decay — `(*fn_ptr)(…)` ≡ `fn_ptr(…)`.

use forge_lexer::Span;
use forge_parser::ast::Expr;
use forge_parser::ast_ops::UnaryOp;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{ImplicitConversion, ParamType, QualType, Type};

use super::helpers::{int, ptr_to, q, ti};

const S: Span = Span::primary(0, 0);

fn ident(name: &str, id: u32) -> Expr {
    Expr::Ident {
        name: name.to_string(),
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

fn call(callee: Expr, id: u32) -> Expr {
    Expr::FunctionCall {
        callee: Box::new(callee),
        args: Vec::new(),
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

fn declare_fn(table: &mut SymbolTable, name: &str, ty: QualType, ctx: &mut SemaContext) {
    let sym = Symbol {
        id: 0,
        name: name.to_string(),
        ty,
        kind: SymbolKind::Function,
        storage: StorageClass::None,
        linkage: Linkage::External,
        span: S,
        is_defined: false,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
    };
    table.declare(sym, ctx).expect("declare must succeed");
}

#[test]
fn addr_of_int_variable_yields_pointer_to_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "x", q(int()), &mut ctx);
    let e = un(UnaryOp::AddrOf, ident("x", 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, ptr_to(q(int())));
}

#[test]
fn addr_of_rvalue_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = un(
        UnaryOp::AddrOf,
        Expr::IntLiteral {
            value: 1,
            suffix: forge_lexer::IntSuffix::None,
            span: S,
            node_id: NodeId(1),
        },
        2,
    );
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("lvalue"));
}

#[test]
fn addr_of_const_variable_yields_pointer_to_const() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let mut qt = q(int());
    qt.is_const = true;
    declare_var(&mut table, "c", qt, &mut ctx);
    let e = un(UnaryOp::AddrOf, ident("c", 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    match qt.ty {
        Type::Pointer { pointee } => {
            assert_eq!(pointee.ty, int());
            assert!(pointee.is_const);
        }
        other => panic!("expected pointer, got {other:?}"),
    }
}

#[test]
fn deref_of_int_pointer_yields_int_lvalue() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    let e = un(UnaryOp::Deref, ident("p", 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    // After default conversions the rvalue is int, but the raw
    // dereference site must have been marked as lvalue first, which in
    // turn triggered LvalueToRvalue.
    assert_eq!(qt.ty, int());
    assert!(ctx.is_lvalue(NodeId(2)));
    assert_eq!(
        ctx.implicit_convs.get(&2),
        Some(&ImplicitConversion::LvalueToRvalue)
    );
}

#[test]
fn deref_of_non_pointer_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "x", q(int()), &mut ctx);
    let e = un(UnaryOp::Deref, ident("x", 1), 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("pointer"));
}

#[test]
fn deref_of_function_pointer_calls_like_raw_function() {
    // `int (*f)(void);  (*f)();` — dereferencing the function pointer
    // yields a function designator that immediately decays back to a
    // pointer; the call should type-check without error and yield int.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let fn_ty = q(Type::Function {
        return_type: Box::new(q(int())),
        params: Vec::<ParamType>::new(),
        is_variadic: false,
        is_prototype: true,
    });
    declare_var(&mut table, "f", q(ptr_to(fn_ty)), &mut ctx);

    let callee = un(UnaryOp::Deref, ident("f", 1), 2);
    let expr = call(callee, 3);
    let qt = check_expr(&expr, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, int());
}

#[test]
fn addr_of_function_is_pointer_to_function() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let fn_ty = Type::Function {
        return_type: Box::new(q(int())),
        params: Vec::<ParamType>::new(),
        is_variadic: false,
        is_prototype: true,
    };
    declare_fn(&mut table, "g", q(fn_ty.clone()), &mut ctx);

    let e = un(UnaryOp::AddrOf, ident("g", 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    // &g has type `int (*)(void)` — pointer to function.
    match qt.ty {
        Type::Pointer { pointee } => assert!(matches!(pointee.ty, Type::Function { .. })),
        other => panic!("expected pointer to function, got {other:?}"),
    }
}
