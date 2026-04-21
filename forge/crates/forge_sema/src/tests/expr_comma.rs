//! Tests for the comma operator (Prompt 4.5).
//!
//! A comma expression type-checks every operand (for side effects) and
//! returns the type and value of the *last* operand.  Type errors in
//! earlier operands are still reported; only the final operand's type
//! survives.

use forge_lexer::{FloatSuffix, IntSuffix, Span};
use forge_parser::ast::Expr;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::SymbolTable;
use crate::types::Type;

use super::helpers::ti;

const S: Span = Span::primary(0, 0);

fn int_lit(v: u64, id: u32) -> Expr {
    Expr::IntLiteral {
        value: v,
        suffix: IntSuffix::None,
        span: S,
        node_id: NodeId(id),
    }
}

fn float_lit(v: f64, id: u32) -> Expr {
    Expr::FloatLiteral {
        value: v,
        suffix: FloatSuffix::None,
        span: S,
        node_id: NodeId(id),
    }
}

fn comma(exprs: Vec<Expr>, id: u32) -> Expr {
    Expr::Comma {
        exprs,
        span: S,
        node_id: NodeId(id),
    }
}

#[test]
fn comma_returns_type_of_last_operand() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = comma(vec![int_lit(1, 1), int_lit(2, 2), float_lit(1.5, 3)], 4);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Double);
}

#[test]
fn comma_with_single_expression_yields_its_type() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let e = comma(vec![int_lit(7, 1)], 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Int { is_unsigned: false });
}

#[test]
fn comma_typechecks_every_operand() {
    // Use an early operand that would be an error in isolation — a
    // `_Generic` with no match — to prove the earlier operand is still
    // type-checked.  We use a simple unary `*` on an int instead,
    // since that is a straightforward error.
    use forge_parser::ast_ops::UnaryOp;
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let bad = Expr::UnaryOp {
        op: UnaryOp::Deref,
        operand: Box::new(int_lit(5, 1)),
        span: S,
        node_id: NodeId(2),
    };
    let e = comma(vec![bad, int_lit(7, 3)], 4);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    // Earlier operand error is emitted, but the comma still yields the
    // last operand's type.
    assert!(ctx.has_errors());
    assert_eq!(qt.ty, Type::Int { is_unsigned: false });
}
