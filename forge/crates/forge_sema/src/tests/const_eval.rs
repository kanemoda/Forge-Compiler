//! Tests for the integer constant expression evaluator.
//!
//! Each test constructs a parser-level [`Expr`] by hand (bypassing the
//! parser) and verifies that [`eval_icx`] returns the expected value or
//! emits a diagnostic.

use forge_lexer::{CharPrefix, FloatSuffix, IntSuffix, Span};
use forge_parser::ast::{
    AbstractDeclarator, DeclSpecifiers, DirectAbstractDeclarator, Expr, TypeName,
    TypeSpecifierToken,
};
use forge_parser::ast_ops::{BinaryOp, UnaryOp};
use forge_parser::node_id::NodeId;

use crate::const_eval::{eval_icx, eval_icx_as_i64, ConstValue};
use crate::context::SemaContext;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::EnumTypeId;

use super::helpers::{int, q, ti};

const S: Span = Span::primary(0, 0);
const N: NodeId = NodeId::DUMMY;

// ---------------------------------------------------------------------
// Small builders
// ---------------------------------------------------------------------

fn lit(value: u64) -> Expr {
    Expr::IntLiteral {
        value,
        suffix: IntSuffix::None,
        span: S,
        node_id: N,
    }
}

fn ulit(value: u64) -> Expr {
    Expr::IntLiteral {
        value,
        suffix: IntSuffix::U,
        span: S,
        node_id: N,
    }
}

fn binop(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::BinaryOp {
        op,
        left: Box::new(left),
        right: Box::new(right),
        span: S,
        node_id: N,
    }
}

fn unop(op: UnaryOp, operand: Expr) -> Expr {
    Expr::UnaryOp {
        op,
        operand: Box::new(operand),
        span: S,
        node_id: N,
    }
}

fn ternary(cond: Expr, t: Expr, e: Expr) -> Expr {
    Expr::Conditional {
        condition: Box::new(cond),
        then_expr: Box::new(t),
        else_expr: Box::new(e),
        span: S,
        node_id: N,
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

fn type_name(specs: DeclSpecifiers) -> TypeName {
    TypeName {
        specifiers: specs,
        abstract_declarator: None,
        span: S,
        node_id: N,
    }
}

fn type_name_array(specs: DeclSpecifiers, size: u64) -> TypeName {
    let abs = AbstractDeclarator {
        pointers: Vec::new(),
        direct: Some(DirectAbstractDeclarator::Array {
            base: None,
            size: forge_parser::ast::ArraySize::Expr(Box::new(lit(size))),
            span: S,
        }),
        span: S,
    };
    TypeName {
        specifiers: specs,
        abstract_declarator: Some(abs),
        span: S,
        node_id: N,
    }
}

fn eval(e: &Expr) -> (Option<ConstValue>, SemaContext) {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let v = eval_icx(e, &mut table, &ti(), &mut ctx);
    (v, ctx)
}

fn eval_ok_i64(e: &Expr) -> i64 {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let v = eval_icx_as_i64(e, &mut table, &ti(), &mut ctx).expect("eval_icx_as_i64");
    assert!(
        !ctx.has_errors(),
        "unexpected diagnostics: {:?}",
        ctx.diagnostics
    );
    v
}

// ---------------------------------------------------------------------
// Arithmetic
// ---------------------------------------------------------------------

#[test]
fn two_plus_three_is_five() {
    assert_eq!(eval_ok_i64(&binop(BinaryOp::Add, lit(2), lit(3))), 5);
}

#[test]
fn subtraction_and_multiplication() {
    // (10 - 3) * 2 = 14
    let e = binop(BinaryOp::Mul, binop(BinaryOp::Sub, lit(10), lit(3)), lit(2));
    assert_eq!(eval_ok_i64(&e), 14);
}

#[test]
fn signed_overflow_wraps() {
    // i64::MAX + 1 wraps to i64::MIN
    let max = Expr::IntLiteral {
        value: i64::MAX as u64,
        suffix: IntSuffix::None,
        span: S,
        node_id: N,
    };
    let e = binop(BinaryOp::Add, max, lit(1));
    assert_eq!(eval_ok_i64(&e), i64::MIN);
}

// ---------------------------------------------------------------------
// Shifts
// ---------------------------------------------------------------------

#[test]
fn one_shl_ten_is_1024() {
    assert_eq!(eval_ok_i64(&binop(BinaryOp::Shl, lit(1), lit(10))), 1024);
}

#[test]
fn shr_on_unsigned_is_logical() {
    // 0xF0 >> 4 == 0x0F on unsigned
    let e = binop(BinaryOp::Shr, ulit(0xF0), lit(4));
    let (v, ctx) = eval(&e);
    assert!(!ctx.has_errors());
    assert_eq!(v, Some(ConstValue::Unsigned(0x0F)));
}

// ---------------------------------------------------------------------
// Bitwise / unary
// ---------------------------------------------------------------------

#[test]
fn unary_minus() {
    let e = unop(UnaryOp::Minus, lit(5));
    assert_eq!(eval_ok_i64(&e), -5);
}

#[test]
fn bitnot_of_zero_is_minus_one() {
    let e = unop(UnaryOp::BitNot, lit(0));
    assert_eq!(eval_ok_i64(&e), -1);
}

#[test]
fn lognot_of_zero_is_one() {
    let e = unop(UnaryOp::LogNot, lit(0));
    assert_eq!(eval_ok_i64(&e), 1);
}

#[test]
fn bitwise_xor() {
    // 0xF0 ^ 0x0F = 0xFF
    let e = binop(BinaryOp::BitXor, lit(0xF0), lit(0x0F));
    assert_eq!(eval_ok_i64(&e), 0xFF);
}

// ---------------------------------------------------------------------
// Logical and ternary short-circuit
// ---------------------------------------------------------------------

#[test]
fn logical_and_short_circuits_on_false() {
    // 0 && (/* would trigger an error */ f()) -> 0, no diagnostic.
    let call = Expr::FunctionCall {
        callee: Box::new(Expr::Ident {
            name: "f".into(),
            span: S,
            node_id: N,
        }),
        args: Vec::new(),
        span: S,
        node_id: N,
    };
    let e = binop(BinaryOp::LogAnd, lit(0), call);
    let (v, ctx) = eval(&e);
    assert_eq!(v, Some(ConstValue::Integer(0)));
    assert!(
        !ctx.has_errors(),
        "short-circuit should suppress the right operand error"
    );
}

#[test]
fn logical_or_short_circuits_on_true() {
    let call = Expr::FunctionCall {
        callee: Box::new(Expr::Ident {
            name: "f".into(),
            span: S,
            node_id: N,
        }),
        args: Vec::new(),
        span: S,
        node_id: N,
    };
    let e = binop(BinaryOp::LogOr, lit(1), call);
    let (v, ctx) = eval(&e);
    assert_eq!(v, Some(ConstValue::Integer(1)));
    assert!(!ctx.has_errors());
}

#[test]
fn ternary_picks_then_branch_when_true() {
    // (1 ? 7 : 8) + 1 == 8
    let t = ternary(lit(1), lit(7), lit(8));
    let e = binop(BinaryOp::Add, t, lit(1));
    assert_eq!(eval_ok_i64(&e), 8);
}

#[test]
fn ternary_picks_else_branch_when_false() {
    let t = ternary(lit(0), lit(7), lit(8));
    assert_eq!(eval_ok_i64(&t), 8);
}

// ---------------------------------------------------------------------
// Cast
// ---------------------------------------------------------------------

#[test]
fn cast_float_to_int_truncates() {
    // (int)2.75 → 2
    let f = Expr::FloatLiteral {
        value: 2.75,
        suffix: FloatSuffix::None,
        span: S,
        node_id: N,
    };
    let cast = Expr::Cast {
        type_name: Box::new(type_name(specs(vec![TypeSpecifierToken::Int]))),
        expr: Box::new(f),
        span: S,
        node_id: N,
    };
    assert_eq!(eval_ok_i64(&cast), 2);
}

#[test]
fn cast_large_int_to_char_truncates_and_sign_extends() {
    // (signed char)257 → 1
    let cast = Expr::Cast {
        type_name: Box::new(type_name(specs(vec![
            TypeSpecifierToken::Signed,
            TypeSpecifierToken::Char,
        ]))),
        expr: Box::new(lit(257)),
        span: S,
        node_id: N,
    };
    assert_eq!(eval_ok_i64(&cast), 1);
}

#[test]
fn cast_to_non_integer_type_errors() {
    let f = Expr::FloatLiteral {
        value: 1.0,
        suffix: FloatSuffix::None,
        span: S,
        node_id: N,
    };
    let cast = Expr::Cast {
        type_name: Box::new(type_name(specs(vec![TypeSpecifierToken::Double]))),
        expr: Box::new(f),
        span: S,
        node_id: N,
    };
    let (v, ctx) = eval(&cast);
    assert!(v.is_none());
    assert!(ctx.has_errors());
}

// ---------------------------------------------------------------------
// sizeof / _Alignof
// ---------------------------------------------------------------------

#[test]
fn sizeof_int_is_four() {
    let e = Expr::SizeofType {
        type_name: Box::new(type_name(specs(vec![TypeSpecifierToken::Int]))),
        span: S,
        node_id: N,
    };
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let v = eval_icx(&e, &mut table, &ti(), &mut ctx).unwrap();
    assert_eq!(v, ConstValue::Unsigned(4));
    assert!(!ctx.has_errors());
}

#[test]
fn sizeof_int_array_of_ten_is_forty() {
    let tn = type_name_array(specs(vec![TypeSpecifierToken::Int]), 10);
    let e = Expr::SizeofType {
        type_name: Box::new(tn),
        span: S,
        node_id: N,
    };
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let v = eval_icx(&e, &mut table, &ti(), &mut ctx).unwrap();
    assert_eq!(v, ConstValue::Unsigned(40));
    assert!(!ctx.has_errors());
}

#[test]
fn alignof_double_is_eight() {
    let e = Expr::AlignofType {
        type_name: Box::new(type_name(specs(vec![TypeSpecifierToken::Double]))),
        span: S,
        node_id: N,
    };
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let v = eval_icx(&e, &mut table, &ti(), &mut ctx).unwrap();
    assert_eq!(v, ConstValue::Unsigned(8));
    assert!(!ctx.has_errors());
}

#[test]
fn sizeof_expr_is_not_yet_supported() {
    let e = Expr::SizeofExpr {
        expr: Box::new(lit(0)),
        span: S,
        node_id: N,
    };
    let (v, ctx) = eval(&e);
    assert!(v.is_none());
    assert!(ctx.has_errors());
}

// ---------------------------------------------------------------------
// Division / modulo by zero
// ---------------------------------------------------------------------

#[test]
fn division_by_zero_emits_diagnostic_and_yields_zero() {
    let e = binop(BinaryOp::Div, lit(1), lit(0));
    let (v, ctx) = eval(&e);
    assert_eq!(v, Some(ConstValue::Integer(0)));
    assert!(ctx.has_errors());
}

#[test]
fn modulo_by_zero_emits_diagnostic_and_yields_zero() {
    let e = binop(BinaryOp::Mod, lit(1), lit(0));
    let (v, ctx) = eval(&e);
    assert_eq!(v, Some(ConstValue::Integer(0)));
    assert!(ctx.has_errors());
}

// ---------------------------------------------------------------------
// Variable refs, function calls — rejected
// ---------------------------------------------------------------------

#[test]
fn reference_to_plain_variable_errors() {
    let e = Expr::Ident {
        name: "x".into(),
        span: S,
        node_id: N,
    };
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table
        .declare(
            Symbol {
                id: 0,
                name: "x".into(),
                ty: q(int()),
                kind: SymbolKind::Variable,
                storage: StorageClass::None,
                linkage: Linkage::None,
                span: S,
                is_defined: true,
                is_inline: false,
                is_noreturn: false,
                has_noreturn_attr: false,
            },
            &mut ctx,
        )
        .unwrap();
    assert!(eval_icx(&e, &mut table, &ti(), &mut ctx).is_none());
    assert!(ctx.has_errors());
}

#[test]
fn function_call_errors() {
    let call = Expr::FunctionCall {
        callee: Box::new(Expr::Ident {
            name: "f".into(),
            span: S,
            node_id: N,
        }),
        args: Vec::new(),
        span: S,
        node_id: N,
    };
    let (v, ctx) = eval(&call);
    assert!(v.is_none());
    assert!(ctx.has_errors());
}

#[test]
fn assignment_errors() {
    let e = Expr::Assignment {
        op: forge_parser::ast_ops::AssignOp::Assign,
        target: Box::new(Expr::Ident {
            name: "x".into(),
            span: S,
            node_id: N,
        }),
        value: Box::new(lit(1)),
        span: S,
        node_id: N,
    };
    let (v, ctx) = eval(&e);
    assert!(v.is_none());
    assert!(ctx.has_errors());
}

#[test]
fn comma_errors() {
    let e = Expr::Comma {
        exprs: vec![lit(1), lit(2)],
        span: S,
        node_id: N,
    };
    let (v, ctx) = eval(&e);
    assert!(v.is_none());
    assert!(ctx.has_errors());
}

// ---------------------------------------------------------------------
// Enum constant reference
// ---------------------------------------------------------------------

#[test]
fn enum_constant_reference_evaluates_to_its_value() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let eid = ctx.type_ctx.fresh_enum_id();
    declare_enum_constant(&mut table, &mut ctx, "A", 5, eid);

    // A + 1 → 6
    let e = binop(
        BinaryOp::Add,
        Expr::Ident {
            name: "A".into(),
            span: S,
            node_id: N,
        },
        lit(1),
    );
    let v = eval_icx_as_i64(&e, &mut table, &ti(), &mut ctx).unwrap();
    assert_eq!(v, 6);
    assert!(!ctx.has_errors());
}

fn declare_enum_constant(
    table: &mut SymbolTable,
    ctx: &mut SemaContext,
    name: &str,
    value: i64,
    enum_id: EnumTypeId,
) {
    table
        .declare(
            Symbol {
                id: 0,
                name: name.to_string(),
                ty: q(int()),
                kind: SymbolKind::EnumConstant { value, enum_id },
                storage: StorageClass::None,
                linkage: Linkage::None,
                span: S,
                is_defined: true,
                is_inline: false,
                is_noreturn: false,
                has_noreturn_attr: false,
            },
            ctx,
        )
        .unwrap();
}

// ---------------------------------------------------------------------
// ConstValue helpers
// ---------------------------------------------------------------------

#[test]
fn const_value_to_i64_handles_unsigned_in_range() {
    assert_eq!(ConstValue::Unsigned(42).to_i64(), Some(42));
}

#[test]
fn const_value_to_i64_rejects_out_of_range_unsigned() {
    assert_eq!(ConstValue::Unsigned(u64::MAX).to_i64(), None);
}

#[test]
fn const_value_to_u64_rejects_negative() {
    assert_eq!(ConstValue::Integer(-1).to_u64(), None);
}

#[test]
fn const_value_is_zero() {
    assert!(ConstValue::Integer(0).is_zero());
    assert!(ConstValue::Unsigned(0).is_zero());
    assert!(ConstValue::Float(0.0).is_zero());
    assert!(!ConstValue::Integer(1).is_zero());
}

// ---------------------------------------------------------------------
// Comparison operators return 0/1
// ---------------------------------------------------------------------

#[test]
fn equality_and_inequality() {
    assert_eq!(eval_ok_i64(&binop(BinaryOp::Eq, lit(2), lit(2))), 1);
    assert_eq!(eval_ok_i64(&binop(BinaryOp::Ne, lit(2), lit(3))), 1);
    assert_eq!(eval_ok_i64(&binop(BinaryOp::Lt, lit(1), lit(2))), 1);
    assert_eq!(eval_ok_i64(&binop(BinaryOp::Ge, lit(3), lit(3))), 1);
}

// ---------------------------------------------------------------------
// Character literal
// ---------------------------------------------------------------------

#[test]
fn character_literal_is_its_code_point() {
    let e = Expr::CharLiteral {
        value: b'A' as u32,
        prefix: CharPrefix::None,
        span: S,
        node_id: N,
    };
    assert_eq!(eval_ok_i64(&e), 65);
}
