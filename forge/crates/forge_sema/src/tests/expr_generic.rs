//! Tests for `_Generic` (Prompt 4.5).
//!
//! The controlling expression is NOT evaluated and is NOT lvalue-
//! converted, but IS array/function decayed.  Exactly one association
//! must match the controller type (via compatibility ignoring
//! qualifiers); if none match and there is no `default:`, it's an
//! error.  If multiple non-default arms match, it's an error.  The
//! type of the `_Generic` expression is the type of the selected arm.

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::{DeclSpecifiers, Expr, GenericAssociation, TypeName, TypeSpecifierToken};
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{QualType, Type};

use super::helpers::{int, q, t_float, ti};

const S: Span = Span::primary(0, 0);
const N: NodeId = NodeId::DUMMY;

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

fn assoc(tn: Option<TypeName>, e: Expr) -> GenericAssociation {
    GenericAssociation {
        type_name: tn,
        expr: Box::new(e),
        span: S,
    }
}

fn generic(controller: Expr, associations: Vec<GenericAssociation>, id: u32) -> Expr {
    Expr::GenericSelection {
        controlling: Box::new(controller),
        associations,
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
fn generic_selects_matching_int_arm() {
    // `_Generic(1, int: 10, double: 20.0)` → int, value type int.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let arms = vec![
        assoc(
            Some(type_name(vec![TypeSpecifierToken::Int])),
            int_lit(10, 1),
        ),
        assoc(
            Some(type_name(vec![TypeSpecifierToken::Double])),
            Expr::FloatLiteral {
                value: 1.5,
                suffix: forge_lexer::FloatSuffix::None,
                span: S,
                node_id: NodeId(2),
            },
        ),
    ];
    let e = generic(int_lit(1, 3), arms, 4);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, int());
}

#[test]
fn generic_selects_default_when_no_match() {
    // `_Generic(1.5f, int: 1, default: 99)` → 99 (arm is int literal).
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "f", q(t_float()), &mut ctx);
    let arms = vec![
        assoc(
            Some(type_name(vec![TypeSpecifierToken::Int])),
            int_lit(1, 1),
        ),
        assoc(None, int_lit(99, 2)),
    ];
    let e = generic(ident("f", 3), arms, 4);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, int());
}

#[test]
fn generic_no_match_no_default_is_error() {
    // `_Generic(1.5f, int: 1)` — float does not match int and no default.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "f", q(t_float()), &mut ctx);
    let arms = vec![assoc(
        Some(type_name(vec![TypeSpecifierToken::Int])),
        int_lit(1, 1),
    )];
    let e = generic(ident("f", 2), arms, 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0]
        .message
        .contains("no _Generic association"));
}

#[test]
fn generic_duplicate_default_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let arms = vec![assoc(None, int_lit(1, 1)), assoc(None, int_lit(2, 2))];
    let e = generic(int_lit(0, 3), arms, 4);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("duplicate default"));
}

#[test]
fn generic_selected_arm_type_is_result() {
    // `_Generic(1, int: 1.5)` — selected arm has type double (via the
    // default `double` classification of the literal).  Result type is
    // double, not int, proving that the arm — not the controller —
    // drives the result type.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let arms = vec![assoc(
        Some(type_name(vec![TypeSpecifierToken::Int])),
        Expr::FloatLiteral {
            value: 1.5,
            suffix: forge_lexer::FloatSuffix::None,
            span: S,
            node_id: NodeId(1),
        },
    )];
    let e = generic(int_lit(1, 2), arms, 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Double);
}
