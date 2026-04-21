//! Tests for identifier type checking (Prompt 4.4).
//!
//! Identifier references consult the symbol table, record the resolved
//! symbol in `symbol_refs`, and mark variables / parameters as lvalues.
//! Enum constants receive type `int` per C17 §6.4.4.3 and are not lvalues.
//! Typedef names and undefined identifiers yield a diagnostic.

use forge_lexer::Span;
use forge_parser::ast::Expr;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{EnumTypeId, QualType, Type};

use super::helpers::{int, q, ti};

const S: Span = Span::primary(0, 0);

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
fn ident_references_existing_variable() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "x", q(int()), &mut ctx);

    let e = ident("x", 1);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
    assert!(ctx.symbol_refs.contains_key(&1));
    assert!(ctx.is_lvalue(NodeId(1)), "variable references are lvalues");
}

#[test]
fn ident_undefined_emits_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let e = ident("nope", 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(
        ctx.diagnostics[0]
            .message
            .contains("undefined identifier 'nope'"),
        "unexpected diagnostic: {:?}",
        ctx.diagnostics[0].message
    );
}

#[test]
fn ident_typedef_emits_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let sym = Symbol {
        id: 0,
        name: "MyInt".to_string(),
        ty: q(int()),
        kind: SymbolKind::Typedef,
        storage: StorageClass::None,
        linkage: Linkage::None,
        span: S,
        is_defined: true,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
    };
    table.declare(sym, &mut ctx).expect("declare must succeed");

    let e = ident("MyInt", 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(
        ctx.diagnostics[0].message.contains("typedef"),
        "unexpected diagnostic: {:?}",
        ctx.diagnostics[0].message
    );
}

#[test]
fn ident_enum_constant_has_int_type_not_lvalue() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let sym = Symbol {
        id: 0,
        name: "RED".to_string(),
        ty: q(Type::Enum(EnumTypeId(0))),
        kind: SymbolKind::EnumConstant {
            value: 0,
            enum_id: EnumTypeId(0),
        },
        storage: StorageClass::None,
        linkage: Linkage::None,
        span: S,
        is_defined: true,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
    };
    table.declare(sym, &mut ctx).expect("declare must succeed");

    let e = ident("RED", 4);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
    assert!(!ctx.is_lvalue(NodeId(4)), "enum constants are not lvalues");
}
