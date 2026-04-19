//! Tests for explicit casts (Prompt 4.5).
//!
//! C17 §6.5.4 bans casting to array types, function types, and
//! struct/union types (the last two also reject `struct` on the
//! *source* side).  Casting an integer to a pointer records a
//! `IntegerToPointer` conversion, and vice versa; arithmetic-to-
//! arithmetic records the usual conversion kind.  A `(void)` cast is
//! always legal and discards the operand's value.

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::{
    AbstractDeclarator, DeclSpecifiers, Expr, PointerQualifiers, TypeName, TypeSpecifierToken,
};
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{ImplicitConversion, QualType, StructLayout, Type};

use super::helpers::{int, q, t_float, ti};

const S: Span = Span::new(0, 0);
const N: NodeId = NodeId::DUMMY;

fn specs(ts: Vec<TypeSpecifierToken>) -> DeclSpecifiers {
    DeclSpecifiers {
        storage_class: None,
        type_specifiers: ts,
        type_qualifiers: Vec::new(),
        function_specifiers: Vec::new(),
        alignment: None,
        attributes: Vec::new(),
        span: S,
    }
}

fn type_name(ts: Vec<TypeSpecifierToken>) -> TypeName {
    TypeName {
        specifiers: specs(ts),
        abstract_declarator: None,
        span: S,
        node_id: N,
    }
}

fn type_name_ptr(ts: Vec<TypeSpecifierToken>) -> TypeName {
    TypeName {
        specifiers: specs(ts),
        abstract_declarator: Some(AbstractDeclarator {
            pointers: vec![PointerQualifiers {
                qualifiers: Vec::new(),
                attributes: Vec::new(),
            }],
            direct: None,
            span: S,
        }),
        span: S,
        node_id: N,
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

fn cast(tn: TypeName, inner: Expr, id: u32) -> Expr {
    Expr::Cast {
        type_name: Box::new(tn),
        expr: Box::new(inner),
        span: S,
        node_id: NodeId(id),
    }
}

fn ident(name: &str, id: u32) -> Expr {
    Expr::Ident {
        name: name.to_string(),
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
fn cast_int_to_float_records_int_to_float_conversion() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = cast(type_name(vec![TypeSpecifierToken::Float]), int_lit(1, 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Float);
    assert_eq!(
        ctx.implicit_convs.get(&1),
        Some(&ImplicitConversion::IntToFloat { to: Type::Float })
    );
}

#[test]
fn cast_int_to_pointer_records_int_to_pointer() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = cast(
        type_name_ptr(vec![TypeSpecifierToken::Int]),
        int_lit(0, 1),
        2,
    );
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert!(matches!(qt.ty, Type::Pointer { .. }));
    assert_eq!(
        ctx.implicit_convs.get(&1),
        Some(&ImplicitConversion::IntegerToPointer)
    );
}

#[test]
fn cast_pointer_to_int_records_pointer_to_integer() {
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
    let e = cast(type_name(vec![TypeSpecifierToken::Long]), ident("p", 1), 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(
        ctx.implicit_convs.get(&1),
        Some(&ImplicitConversion::PointerToInteger)
    );
}

#[test]
fn cast_to_void_is_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = cast(type_name(vec![TypeSpecifierToken::Void]), int_lit(1, 1), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Void);
}

#[test]
fn cast_from_struct_is_error() {
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
    let e = cast(type_name(vec![TypeSpecifierToken::Int]), ident("s", 1), 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("struct"));
}

#[test]
fn cast_float_to_int_records_float_to_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "f", q(t_float()), &mut ctx);
    let e = cast(type_name(vec![TypeSpecifierToken::Int]), ident("f", 1), 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(
        ctx.implicit_convs.get(&1),
        Some(&ImplicitConversion::FloatToInt { to: int() })
    );
}
