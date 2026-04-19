//! Tests for `_Alignof(type-name)` (Prompt 4.4).

use forge_lexer::Span;
use forge_parser::ast::{DeclSpecifiers, Expr, TypeName, TypeSpecifierToken};
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

fn alignof_type(tn: TypeName, id: u32) -> Expr {
    Expr::AlignofType {
        type_name: Box::new(tn),
        span: S,
        node_id: NodeId(id),
    }
}

#[test]
fn alignof_int_yields_size_t() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let e = alignof_type(type_name(vec![TypeSpecifierToken::Int]), 1);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    assert_eq!(qt.ty, Type::Long { is_unsigned: true });
}

#[test]
fn alignof_function_type_is_error() {
    // int(void) — a function type does not have an alignment.
    use forge_parser::ast::{AbstractDeclarator, DirectAbstractDeclarator};

    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let fn_tn = TypeName {
        specifiers: specs(vec![TypeSpecifierToken::Int]),
        abstract_declarator: Some(AbstractDeclarator {
            pointers: Vec::new(),
            direct: Some(DirectAbstractDeclarator::Function {
                base: None,
                params: Vec::new(),
                is_variadic: false,
                span: S,
            }),
            span: S,
        }),
        span: S,
        node_id: N,
    };

    let e = alignof_type(fn_tn, 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("function type"));
}

#[test]
fn alignof_char_is_one() {
    // _Alignof(char) must compute a size_t with value 1.  We can't read
    // the "value" directly (the expression is not a ConstValue return
    // here), but the returned type must be size_t.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let e = alignof_type(type_name(vec![TypeSpecifierToken::Char]), 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Long { is_unsigned: true });
}
