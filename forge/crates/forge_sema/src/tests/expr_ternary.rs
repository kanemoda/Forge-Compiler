//! Tests for the ternary conditional operator (Prompt 4.5).
//!
//! The condition must be scalar; the two result expressions pick a
//! common type via the usual arithmetic conversions (arithmetic case)
//! or pointer composite rules (pointer case).  A null pointer constant
//! on one side and a pointer on the other yields the pointer type; a
//! void pointer on either side yields a pointer to void with the
//! qualifier union.  Struct / union operands must have the same tag.

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::Expr;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{QualType, StructLayout, Type};

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

fn cond(c: Expr, a: Expr, b: Expr, id: u32) -> Expr {
    Expr::Conditional {
        condition: Box::new(c),
        then_expr: Box::new(a),
        else_expr: Box::new(b),
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
        address_taken: false,
    };
    table.declare(sym, ctx).expect("declare must succeed");
}

#[test]
fn ternary_two_ints_yields_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = cond(int_lit(1, 1), int_lit(2, 2), int_lit(3, 3), 4);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
}

#[test]
fn ternary_int_and_double_promotes_to_double() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "d", q(t_double()), &mut ctx);
    let e = cond(int_lit(1, 1), int_lit(2, 2), ident("d", 3), 4);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Double);
}

#[test]
fn ternary_condition_must_be_scalar() {
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
    let e = cond(ident("s", 1), int_lit(1, 2), int_lit(2, 3), 4);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("scalar"));
}

#[test]
fn ternary_pointer_and_null_yields_pointer() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    let e = cond(int_lit(1, 1), ident("p", 2), int_lit(0, 3), 4);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, ptr_to(q(int())));
}

#[test]
fn ternary_pointer_and_void_pointer_yields_void_pointer() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    declare_var(&mut table, "v", q(ptr_to(q(void()))), &mut ctx);
    let e = cond(int_lit(1, 1), ident("p", 2), ident("v", 3), 4);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert!(matches!(qt.ty, Type::Pointer { ref pointee } if matches!(pointee.ty, Type::Void)));
}

#[test]
fn ternary_two_voids_yields_void() {
    // `c ? (void)x : (void)y` — legal, result is void.
    use forge_parser::ast::{AbstractDeclarator, DeclSpecifiers, TypeName, TypeSpecifierToken};
    const N: NodeId = NodeId::DUMMY;
    let specs = DeclSpecifiers {
        storage_class: None,
        type_specifiers: vec![TypeSpecifierToken::Void],
        type_qualifiers: Vec::new(),
        function_specifiers: Vec::new(),
        alignment: None,
        attributes: Vec::new(),
        span: S,
    };
    let void_tn = TypeName {
        specifiers: specs,
        abstract_declarator: None::<AbstractDeclarator>,
        span: S,
        node_id: N,
    };
    let void_cast = |inner, id| Expr::Cast {
        type_name: Box::new(void_tn.clone()),
        expr: Box::new(inner),
        span: S,
        node_id: NodeId(id),
    };
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = cond(
        int_lit(1, 1),
        void_cast(int_lit(1, 2), 3),
        void_cast(int_lit(2, 4), 5),
        6,
    );
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Void);
}

#[test]
fn ternary_same_struct_ok() {
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
    declare_var(&mut table, "a", q(Type::Struct(sid)), &mut ctx);
    declare_var(&mut table, "b", q(Type::Struct(sid)), &mut ctx);
    let e = cond(int_lit(1, 1), ident("a", 2), ident("b", 3), 4);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, Type::Struct(sid));
}

#[test]
fn ternary_incompatible_operands_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "p", q(ptr_to(q(int()))), &mut ctx);
    declare_var(&mut table, "d", q(t_double()), &mut ctx);
    let e = cond(int_lit(1, 1), ident("p", 2), ident("d", 3), 4);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("incompatible"));
}
