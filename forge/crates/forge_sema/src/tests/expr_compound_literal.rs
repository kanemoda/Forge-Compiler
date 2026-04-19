//! Tests for compound literals — `(T){ initializer }` — (Prompt 4.5).
//!
//! A compound literal creates a fresh anonymous object with storage
//! duration that matches its enclosing scope.  Per C17 §6.5.2.5p4 it is
//! an LVALUE — testing this fact matters because it's a common
//! correctness bug in C compilers (a cast-like expression that ALSO
//! behaves as an addressable object).  The initializer is type-checked
//! against the declared type.

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::{
    DeclSpecifiers, DesignatedInit, Expr, Initializer, TypeName, TypeSpecifierToken,
};
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::SymbolTable;
use crate::types::Type;

use super::helpers::ti;

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

fn int_lit(v: u64, id: u32) -> Expr {
    Expr::IntLiteral {
        value: v,
        suffix: IntSuffix::None,
        span: S,
        node_id: NodeId(id),
    }
}

fn compound(tn: TypeName, init: Initializer, id: u32) -> Expr {
    Expr::CompoundLiteral {
        type_name: Box::new(tn),
        initializer: init,
        span: S,
        node_id: NodeId(id),
    }
}

#[test]
fn compound_literal_int_is_lvalue() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let init = Initializer::Expr(Box::new(int_lit(42, 1)));
    let e = compound(type_name(vec![TypeSpecifierToken::Int]), init, 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, Type::Int { is_unsigned: false });
    assert!(
        ctx.is_lvalue(NodeId(2)),
        "a compound literal is an lvalue per C17 §6.5.2.5p4"
    );
}

#[test]
fn compound_literal_with_braced_list_ok() {
    // `(int){ 7 }` — single-element brace init.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let item = DesignatedInit {
        designators: Vec::new(),
        initializer: Box::new(Initializer::Expr(Box::new(int_lit(7, 1)))),
        span: S,
    };
    let init = Initializer::List {
        items: vec![item],
        span: S,
        node_id: N,
    };
    let e = compound(type_name(vec![TypeSpecifierToken::Int]), init, 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, Type::Int { is_unsigned: false });
}
