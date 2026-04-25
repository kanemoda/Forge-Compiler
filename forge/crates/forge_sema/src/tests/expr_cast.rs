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

const S: Span = Span::primary(0, 0);
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
        address_taken: false,
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

// =========================================================================
// `(void)expr` is the explicit-discard cast — C17 §6.5.4p2
// =========================================================================
//
// These tests guard against a regression where the cast checker rejected
// `(void)struct_lvalue` because the "cannot cast from a struct or union
// type" rule fired *before* the void-target short-circuit.

#[test]
fn void_cast_of_struct_lvalue_is_ok() {
    super::helpers::assert_source_clean(
        r#"
            struct S { int a; };
            void f(void) {
                struct S s;
                (void)s;
            }
        "#,
    );
}

#[test]
fn void_cast_of_union_lvalue_is_ok() {
    super::helpers::assert_source_clean(
        r#"
            union U { int i; float f; };
            void f(void) {
                union U u;
                (void)u;
            }
        "#,
    );
}

#[test]
fn void_cast_of_int_is_ok() {
    // Regression guard — this case worked before the fix and must keep
    // working.
    super::helpers::assert_source_clean(
        r#"
            void f(void) {
                int x = 5;
                (void)x;
            }
        "#,
    );
}

#[test]
fn void_cast_of_function_call_is_ok() {
    // The canonical "discard return value" idiom.
    super::helpers::assert_source_clean(
        r#"
            int f(void);
            void g(void) {
                (void)f();
            }
        "#,
    );
}

#[test]
fn void_cast_of_void_is_ok() {
    // Edge case: discarding the (already-void) result of a void
    // function.  No real value to throw away, but the cast must still
    // type-check cleanly.
    super::helpers::assert_source_clean(
        r#"
            void f(void);
            void g(void) {
                (void)f();
            }
        "#,
    );
}

#[test]
fn non_void_cast_of_struct_is_still_error() {
    // Regression guard: the void short-circuit must not accidentally
    // allow a struct source for non-void casts.
    super::helpers::assert_source_has_errors(
        r#"
            struct S { int a; };
            int g(void) {
                struct S s;
                return (int)s;
            }
        "#,
    );
}

#[test]
fn non_void_cast_to_struct_is_still_error() {
    // Regression guard: target struct is forbidden regardless of how
    // the void special case is handled.
    super::helpers::assert_source_has_errors(
        r#"
            struct S { int a; };
            void f(void) {
                int x = 5;
                struct S s = (struct S)x;
                (void)s;
            }
        "#,
    );
}
