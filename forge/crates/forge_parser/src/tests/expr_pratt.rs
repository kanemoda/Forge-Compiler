//! Pratt expression parser tests.

use crate::ast::*;
use crate::ast_ops::*;

use super::helpers::parse_expr;

// =====================================================================
// Helpers for matching
// =====================================================================

/// Unwrap a `BinaryOp` node or panic.
fn bin(e: &Expr) -> (&BinaryOp, &Expr, &Expr) {
    match e {
        Expr::BinaryOp {
            op, left, right, ..
        } => (op, left, right),
        other => panic!("expected BinaryOp, got {other:?}"),
    }
}

/// Unwrap an `Assignment` node or panic.
fn assign(e: &Expr) -> (&AssignOp, &Expr, &Expr) {
    match e {
        Expr::Assignment {
            op, target, value, ..
        } => (op, target, value),
        other => panic!("expected Assignment, got {other:?}"),
    }
}

/// Unwrap a `UnaryOp` node or panic.
fn unary(e: &Expr) -> (&UnaryOp, &Expr) {
    match e {
        Expr::UnaryOp { op, operand, .. } => (op, operand),
        other => panic!("expected UnaryOp, got {other:?}"),
    }
}

/// Unwrap a `PostfixOp` node or panic.
fn postfix(e: &Expr) -> (&PostfixOp, &Expr) {
    match e {
        Expr::PostfixOp { op, operand, .. } => (op, operand),
        other => panic!("expected PostfixOp, got {other:?}"),
    }
}

/// Assert the expression is an Ident with the given name.
fn assert_ident(e: &Expr, expected: &str) {
    match e {
        Expr::Ident { name, .. } => assert_eq!(name, expected),
        other => panic!("expected Ident({expected}), got {other:?}"),
    }
}

/// Assert the expression is an IntLiteral with the given value.
fn assert_int(e: &Expr, expected: u64) {
    match e {
        Expr::IntLiteral { value, .. } => assert_eq!(*value, expected),
        other => panic!("expected IntLiteral({expected}), got {other:?}"),
    }
}

// =====================================================================
// Precedence
// =====================================================================

#[test]
fn add_mul_precedence() {
    // 1 + 2 * 3 → Add(1, Mul(2, 3))
    let e = parse_expr("1 + 2 * 3");
    let (op, left, right) = bin(&e);
    assert_eq!(*op, BinaryOp::Add);
    assert_int(left, 1);
    let (op2, l2, r2) = bin(right);
    assert_eq!(*op2, BinaryOp::Mul);
    assert_int(l2, 2);
    assert_int(r2, 3);
}

#[test]
fn mul_add_precedence() {
    // 1 * 2 + 3 → Add(Mul(1, 2), 3)
    let e = parse_expr("1 * 2 + 3");
    let (op, left, right) = bin(&e);
    assert_eq!(*op, BinaryOp::Add);
    let (op2, l2, r2) = bin(left);
    assert_eq!(*op2, BinaryOp::Mul);
    assert_int(l2, 1);
    assert_int(r2, 2);
    assert_int(right, 3);
}

#[test]
fn add_left_associative() {
    // a + b + c → Add(Add(a, b), c)
    let e = parse_expr("a + b + c");
    let (op, left, right) = bin(&e);
    assert_eq!(*op, BinaryOp::Add);
    assert_ident(right, "c");
    let (op2, l2, r2) = bin(left);
    assert_eq!(*op2, BinaryOp::Add);
    assert_ident(l2, "a");
    assert_ident(r2, "b");
}

#[test]
fn sub_left_associative() {
    // a - b - c → Sub(Sub(a, b), c)
    let e = parse_expr("a - b - c");
    let (op, left, right) = bin(&e);
    assert_eq!(*op, BinaryOp::Sub);
    assert_ident(right, "c");
    let (op2, l2, r2) = bin(left);
    assert_eq!(*op2, BinaryOp::Sub);
    assert_ident(l2, "a");
    assert_ident(r2, "b");
}

#[test]
fn assign_right_associative() {
    // a = b = c → Assign(a, Assign(b, c))
    let e = parse_expr("a = b = c");
    let (op, target, value) = assign(&e);
    assert_eq!(*op, AssignOp::Assign);
    assert_ident(target, "a");
    let (op2, t2, v2) = assign(value);
    assert_eq!(*op2, AssignOp::Assign);
    assert_ident(t2, "b");
    assert_ident(v2, "c");
}

// =====================================================================
// Ternary
// =====================================================================

#[test]
fn ternary_simple() {
    // a ? b : c → Conditional(a, b, c)
    let e = parse_expr("a ? b : c");
    match &e {
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            assert_ident(condition, "a");
            assert_ident(then_expr, "b");
            assert_ident(else_expr, "c");
        }
        other => panic!("expected Conditional, got {other:?}"),
    }
}

#[test]
fn ternary_right_associative() {
    // a ? b : c ? d : e → Conditional(a, b, Conditional(c, d, e))
    let e = parse_expr("a ? b : c ? d : e");
    match &e {
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            assert_ident(condition, "a");
            assert_ident(then_expr, "b");
            match else_expr.as_ref() {
                Expr::Conditional {
                    condition: c2,
                    then_expr: t2,
                    else_expr: e2,
                    ..
                } => {
                    assert_ident(c2, "c");
                    assert_ident(t2, "d");
                    assert_ident(e2, "e");
                }
                other => panic!("expected nested Conditional, got {other:?}"),
            }
        }
        other => panic!("expected Conditional, got {other:?}"),
    }
}

// =====================================================================
// Unary
// =====================================================================

#[test]
fn unary_minus() {
    // -x → UnaryOp(Minus, Ident(x))
    let e = parse_expr("-x");
    let (op, operand) = unary(&e);
    assert_eq!(*op, UnaryOp::Minus);
    assert_ident(operand, "x");
}

#[test]
fn logical_not_and() {
    // !a && b → LogAnd(LogNot(a), b)
    let e = parse_expr("!a && b");
    let (op, left, right) = bin(&e);
    assert_eq!(*op, BinaryOp::LogAnd);
    assert_ident(right, "b");
    let (uop, inner) = unary(left);
    assert_eq!(*uop, UnaryOp::LogNot);
    assert_ident(inner, "a");
}

#[test]
fn deref_post_increment() {
    // *p++ → Deref(PostIncrement(p))
    let e = parse_expr("*p++");
    let (op, operand) = unary(&e);
    assert_eq!(*op, UnaryOp::Deref);
    let (pop, inner) = postfix(operand);
    assert_eq!(*pop, PostfixOp::PostIncrement);
    assert_ident(inner, "p");
}

#[test]
fn chained_unary() {
    // -!x → Minus(LogNot(x))
    let e = parse_expr("-!x");
    let (op, operand) = unary(&e);
    assert_eq!(*op, UnaryOp::Minus);
    let (op2, inner) = unary(operand);
    assert_eq!(*op2, UnaryOp::LogNot);
    assert_ident(inner, "x");
}

#[test]
fn pre_increment_deref() {
    // ++*p → PreIncrement(Deref(p))
    let e = parse_expr("++*p");
    let (op, operand) = unary(&e);
    assert_eq!(*op, UnaryOp::PreIncrement);
    let (op2, inner) = unary(operand);
    assert_eq!(*op2, UnaryOp::Deref);
    assert_ident(inner, "p");
}

// =====================================================================
// Postfix
// =====================================================================

#[test]
fn array_subscript() {
    // a[0] → ArraySubscript(a, 0)
    let e = parse_expr("a[0]");
    match &e {
        Expr::ArraySubscript { array, index, .. } => {
            assert_ident(array, "a");
            assert_int(index, 0);
        }
        other => panic!("expected ArraySubscript, got {other:?}"),
    }
}

#[test]
fn member_access_dot() {
    // a.b → MemberAccess(a, "b", false)
    let e = parse_expr("a.b");
    match &e {
        Expr::MemberAccess {
            object,
            member,
            is_arrow,
            ..
        } => {
            assert_ident(object, "a");
            assert_eq!(member, "b");
            assert!(!is_arrow);
        }
        other => panic!("expected MemberAccess, got {other:?}"),
    }
}

#[test]
fn member_access_arrow() {
    // a->b → MemberAccess(a, "b", true)
    let e = parse_expr("a->b");
    match &e {
        Expr::MemberAccess {
            object,
            member,
            is_arrow,
            ..
        } => {
            assert_ident(object, "a");
            assert_eq!(member, "b");
            assert!(is_arrow);
        }
        other => panic!("expected MemberAccess, got {other:?}"),
    }
}

#[test]
fn chained_postfix_left_to_right() {
    // a[0].b->c++ → PostIncrement(Arrow(Dot(Subscript(a, 0), b), c))
    let e = parse_expr("a[0].b->c++");
    let (pop, inner) = postfix(&e);
    assert_eq!(*pop, PostfixOp::PostIncrement);
    match inner {
        Expr::MemberAccess {
            object,
            member,
            is_arrow,
            ..
        } => {
            assert_eq!(member, "c");
            assert!(is_arrow);
            match object.as_ref() {
                Expr::MemberAccess {
                    object: obj2,
                    member: m2,
                    is_arrow: arr2,
                    ..
                } => {
                    assert_eq!(m2, "b");
                    assert!(!arr2);
                    match obj2.as_ref() {
                        Expr::ArraySubscript { array, index, .. } => {
                            assert_ident(array, "a");
                            assert_int(index, 0);
                        }
                        other => panic!("expected ArraySubscript, got {other:?}"),
                    }
                }
                other => panic!("expected MemberAccess, got {other:?}"),
            }
        }
        other => panic!("expected MemberAccess, got {other:?}"),
    }
}

#[test]
fn function_call_with_args() {
    // f(a, b, c) → FunctionCall(f, [a, b, c])
    let e = parse_expr("f(a, b, c)");
    match &e {
        Expr::FunctionCall { callee, args, .. } => {
            assert_ident(callee, "f");
            assert_eq!(args.len(), 3);
            assert_ident(&args[0], "a");
            assert_ident(&args[1], "b");
            assert_ident(&args[2], "c");
        }
        other => panic!("expected FunctionCall, got {other:?}"),
    }
}

#[test]
fn function_call_no_args() {
    // f() → FunctionCall(f, [])
    let e = parse_expr("f()");
    match &e {
        Expr::FunctionCall { callee, args, .. } => {
            assert_ident(callee, "f");
            assert!(args.is_empty());
        }
        other => panic!("expected FunctionCall, got {other:?}"),
    }
}

// =====================================================================
// Cast (minimal type-name)
// =====================================================================

#[test]
fn cast_int() {
    // (int)x → Cast(Int, x)
    let e = parse_expr("(int)x");
    match &e {
        Expr::Cast {
            type_name, expr, ..
        } => {
            assert_eq!(type_name.specifiers.type_specifiers.len(), 1);
            assert!(matches!(
                type_name.specifiers.type_specifiers[0],
                TypeSpecifierToken::Int
            ));
            assert_ident(expr, "x");
        }
        other => panic!("expected Cast, got {other:?}"),
    }
}

#[test]
fn cast_unsigned_long() {
    // (unsigned long)x → Cast([Unsigned, Long], x)
    let e = parse_expr("(unsigned long)x");
    match &e {
        Expr::Cast {
            type_name, expr, ..
        } => {
            assert_eq!(type_name.specifiers.type_specifiers.len(), 2);
            assert!(matches!(
                type_name.specifiers.type_specifiers[0],
                TypeSpecifierToken::Unsigned
            ));
            assert!(matches!(
                type_name.specifiers.type_specifiers[1],
                TypeSpecifierToken::Long
            ));
            assert_ident(expr, "x");
        }
        other => panic!("expected Cast, got {other:?}"),
    }
}

#[test]
fn cast_chain() {
    // (int *)(void *)p → Cast(int*, Cast(void*, p))
    let e = parse_expr("(int *)(void *)p");
    match &e {
        Expr::Cast {
            type_name: outer_tn,
            expr: inner_cast,
            ..
        } => {
            assert!(matches!(
                outer_tn.specifiers.type_specifiers[0],
                TypeSpecifierToken::Int
            ));
            assert!(outer_tn.abstract_declarator.is_some());
            match inner_cast.as_ref() {
                Expr::Cast {
                    type_name: inner_tn,
                    expr: innermost,
                    ..
                } => {
                    assert!(matches!(
                        inner_tn.specifiers.type_specifiers[0],
                        TypeSpecifierToken::Void
                    ));
                    assert_ident(innermost, "p");
                }
                other => panic!("expected inner Cast, got {other:?}"),
            }
        }
        other => panic!("expected Cast, got {other:?}"),
    }
}

// =====================================================================
// sizeof
// =====================================================================

#[test]
fn sizeof_type() {
    // sizeof(int) → SizeofType(Int)
    let e = parse_expr("sizeof(int)");
    match &e {
        Expr::SizeofType { type_name, .. } => {
            assert!(matches!(
                type_name.specifiers.type_specifiers[0],
                TypeSpecifierToken::Int
            ));
        }
        other => panic!("expected SizeofType, got {other:?}"),
    }
}

#[test]
fn sizeof_expr_bare() {
    // sizeof x → SizeofExpr(Ident(x))
    let e = parse_expr("sizeof x");
    match &e {
        Expr::SizeofExpr { expr, .. } => {
            assert_ident(expr, "x");
        }
        other => panic!("expected SizeofExpr, got {other:?}"),
    }
}

#[test]
fn sizeof_expr_with_parens() {
    // sizeof(x) where x is NOT a typedef → SizeofExpr
    let e = parse_expr("sizeof(x)");
    match &e {
        Expr::SizeofExpr { expr, .. } => {
            // The inner expression is x (the parens are transparent).
            assert_ident(expr, "x");
        }
        other => panic!("expected SizeofExpr, got {other:?}"),
    }
}

// =====================================================================
// Compound literal (minimal)
// =====================================================================

#[test]
fn compound_literal_int() {
    // (int){42} → CompoundLiteral(Int, [42])
    let e = parse_expr("(int){42}");
    match &e {
        Expr::CompoundLiteral {
            type_name,
            initializer,
            ..
        } => {
            assert!(matches!(
                type_name.specifiers.type_specifiers[0],
                TypeSpecifierToken::Int
            ));
            match initializer {
                Initializer::List { items, .. } => {
                    assert_eq!(items.len(), 1);
                    match items[0].initializer.as_ref() {
                        Initializer::Expr(e) => assert_int(e, 42),
                        other => panic!("expected Initializer::Expr, got {other:?}"),
                    }
                }
                other => panic!("expected Initializer::List, got {other:?}"),
            }
        }
        other => panic!("expected CompoundLiteral, got {other:?}"),
    }
}

// =====================================================================
// String concatenation
// =====================================================================

#[test]
fn string_concatenation() {
    // "hello" " " "world" → StringLiteral("hello world")
    let e = parse_expr("\"hello\" \" \" \"world\"");
    match &e {
        Expr::StringLiteral { value, .. } => {
            assert_eq!(value, "hello world");
        }
        other => panic!("expected StringLiteral, got {other:?}"),
    }
}

// =====================================================================
// Comma
// =====================================================================

#[test]
fn comma_expression() {
    // a, b, c → Comma([a, b, c])
    let e = parse_expr("a, b, c");
    match &e {
        Expr::Comma { exprs, .. } => {
            assert_eq!(exprs.len(), 3);
            assert_ident(&exprs[0], "a");
            assert_ident(&exprs[1], "b");
            assert_ident(&exprs[2], "c");
        }
        other => panic!("expected Comma, got {other:?}"),
    }
}

// =====================================================================
// Complex / mixed precedence
// =====================================================================

#[test]
fn complex_assign_call() {
    // *p++ = f(a + b, c) → Assign(Deref(PostInc(p)), FunctionCall(f, [Add(a,b), c]))
    let e = parse_expr("*p++ = f(a + b, c)");
    let (aop, target, value) = assign(&e);
    assert_eq!(*aop, AssignOp::Assign);
    // target: Deref(PostIncrement(p))
    let (uop, inner) = unary(target);
    assert_eq!(*uop, UnaryOp::Deref);
    let (pop, innermost) = postfix(inner);
    assert_eq!(*pop, PostfixOp::PostIncrement);
    assert_ident(innermost, "p");
    // value: FunctionCall(f, [Add(a,b), c])
    match value {
        Expr::FunctionCall { callee, args, .. } => {
            assert_ident(callee, "f");
            assert_eq!(args.len(), 2);
            let (bop, l, r) = bin(&args[0]);
            assert_eq!(*bop, BinaryOp::Add);
            assert_ident(l, "a");
            assert_ident(r, "b");
            assert_ident(&args[1], "c");
        }
        other => panic!("expected FunctionCall, got {other:?}"),
    }
}

#[test]
fn logical_or_and_precedence() {
    // a || b && c → LogOr(a, LogAnd(b, c))
    let e = parse_expr("a || b && c");
    let (op, left, right) = bin(&e);
    assert_eq!(*op, BinaryOp::LogOr);
    assert_ident(left, "a");
    let (op2, l2, r2) = bin(right);
    assert_eq!(*op2, BinaryOp::LogAnd);
    assert_ident(l2, "b");
    assert_ident(r2, "c");
}

#[test]
fn bitand_eq_precedence() {
    // a & b == c → BitAnd(a, Eq(b, c))
    let e = parse_expr("a & b == c");
    let (op, left, right) = bin(&e);
    assert_eq!(*op, BinaryOp::BitAnd);
    assert_ident(left, "a");
    let (op2, l2, r2) = bin(right);
    assert_eq!(*op2, BinaryOp::Eq);
    assert_ident(l2, "b");
    assert_ident(r2, "c");
}

// =====================================================================
// _Generic (minimal type-name)
// =====================================================================

#[test]
fn generic_selection() {
    // _Generic(x, int: 1, float: 2, default: 0)
    let e = parse_expr("_Generic(x, int: 1, float: 2, default: 0)");
    match &e {
        Expr::GenericSelection {
            controlling,
            associations,
            ..
        } => {
            assert_ident(controlling, "x");
            assert_eq!(associations.len(), 3);
            // int: 1
            assert!(associations[0].type_name.is_some());
            assert_int(&associations[0].expr, 1);
            // float: 2
            assert!(associations[1].type_name.is_some());
            assert_int(&associations[1].expr, 2);
            // default: 0
            assert!(associations[2].type_name.is_none());
            assert_int(&associations[2].expr, 0);
        }
        other => panic!("expected GenericSelection, got {other:?}"),
    }
}
