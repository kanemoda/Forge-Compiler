//! Tests for assignment and compound-assignment operators (Prompt 4.5).
//!
//! Simple `=` checks that the LHS is a modifiable lvalue and that the
//! RHS is assignable to the LHS type.  Pointer assignment enforces
//! compatibility, permits void* interop, rejects qualifier discard, and
//! accepts null pointer constants.  Struct assignment requires matching
//! tags.  The result type of any assignment is the LHS type UNQUALIFIED.

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::Expr;
use forge_parser::ast_ops::AssignOp;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{ImplicitConversion, QualType, StructLayout, Type};

use super::helpers::{int, ptr_to, q, t_bool, t_double, t_float, ti, void};

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

fn assign(op: AssignOp, target: Expr, value: Expr, id: u32) -> Expr {
    Expr::Assignment {
        op,
        target: Box::new(target),
        value: Box::new(value),
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
fn assign_int_to_int_variable_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "x", q(int()), &mut ctx);
    let e = assign(AssignOp::Assign, ident("x", 1), int_lit(42, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, int());
    // Result must be unqualified, never an lvalue.
    assert!(!ctx.is_lvalue(NodeId(3)));
}

#[test]
fn assign_to_rvalue_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = assign(AssignOp::Assign, int_lit(1, 1), int_lit(2, 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("lvalue"));
}

#[test]
fn assign_to_const_lvalue_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let mut qt = q(int());
    qt.is_const = true;
    declare_var(&mut table, "c", qt, &mut ctx);
    let e = assign(AssignOp::Assign, ident("c", 1), int_lit(1, 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("const"));
}

#[test]
fn assign_float_to_int_records_float_to_int_conversion() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "x", q(int()), &mut ctx);
    declare_var(&mut table, "f", q(t_float()), &mut ctx);
    let e = assign(AssignOp::Assign, ident("x", 1), ident("f", 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(
        ctx.implicit_convs.get(&2),
        Some(&ImplicitConversion::FloatToInt { to: int() })
    );
}

#[test]
fn assign_pointer_to_bool_records_pointer_to_boolean() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "b", q(t_bool()), &mut ctx);
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    let e = assign(AssignOp::Assign, ident("b", 1), ident("p", 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(
        ctx.implicit_convs.get(&2),
        Some(&ImplicitConversion::PointerToBoolean)
    );
}

#[test]
fn assign_null_constant_to_pointer_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    let e = assign(AssignOp::Assign, ident("p", 1), int_lit(0, 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(
        ctx.implicit_convs.get(&2),
        Some(&ImplicitConversion::NullPointerConversion)
    );
}

#[test]
fn assign_incompatible_pointer_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "pi", q(ptr_to(q(int()))), &mut ctx);
    declare_var(&mut table, "pd", q(ptr_to(q(t_double()))), &mut ctx);
    let e = assign(AssignOp::Assign, ident("pi", 1), ident("pd", 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("incompatible"));
}

#[test]
fn assign_void_pointer_to_int_pointer_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    declare_var(&mut table, "v", q(ptr_to(q(void()))), &mut ctx);
    let e = assign(AssignOp::Assign, ident("p", 1), ident("v", 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
}

#[test]
fn assign_pointer_to_const_from_pointer_to_non_const_ok() {
    // `const int *p = q;` where q is `int *` — LHS acquires qualifiers.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let mut const_int = q(int());
    const_int.is_const = true;
    declare_var(&mut table, "p", q(ptr_to(const_int)), &mut ctx);
    declare_var(&mut table, "q", q(ptr_to(q(int()))), &mut ctx);
    let e = assign(AssignOp::Assign, ident("p", 1), ident("q", 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(
        ctx.implicit_convs.get(&2),
        Some(&ImplicitConversion::QualificationConversion)
    );
}

#[test]
fn assign_pointer_to_non_const_from_pointer_to_const_is_error() {
    // The RHS qualifiers are NOT a subset of the LHS qualifiers.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let mut const_int = q(int());
    const_int.is_const = true;
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    declare_var(&mut table, "q", q(ptr_to(const_int)), &mut ctx);
    let e = assign(AssignOp::Assign, ident("p", 1), ident("q", 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("discards qualifiers"));
}

#[test]
fn compound_mod_assign_rejects_float() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "f", q(t_float()), &mut ctx);
    let e = assign(AssignOp::ModAssign, ident("f", 1), int_lit(2, 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("integer"));
}

#[test]
fn compound_add_assign_on_int_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "x", q(int()), &mut ctx);
    let e = assign(AssignOp::AddAssign, ident("x", 1), int_lit(1, 2), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
}

#[test]
fn assign_struct_with_different_tag_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let sa = ctx.type_ctx.fresh_struct_id();
    let sb = ctx.type_ctx.fresh_struct_id();
    for sid in [sa, sb] {
        ctx.type_ctx.set_struct(
            sid,
            StructLayout {
                is_complete: true,
                total_size: 4,
                alignment: 4,
                ..StructLayout::default()
            },
        );
    }
    declare_var(&mut table, "a", q(Type::Struct(sa)), &mut ctx);
    declare_var(&mut table, "b", q(Type::Struct(sb)), &mut ctx);
    let e = assign(AssignOp::Assign, ident("a", 1), ident("b", 2), 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("same tag"));
}
