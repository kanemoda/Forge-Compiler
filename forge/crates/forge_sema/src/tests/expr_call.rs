//! Tests for function calls (Prompt 4.5).
//!
//! Callees decay from function-designator to pointer-to-function.  A
//! prototyped call checks arity and argument assignability; variadic
//! tail arguments get the default argument promotions (char/short/bool
//! → int, float → double).  The return type is returned with its
//! top-level qualifiers stripped per C17 §6.5.2.2p5.  An unprototyped
//! call emits a warning but still type-checks.

use forge_diagnostics::Severity;
use forge_lexer::{FloatSuffix, IntSuffix, Span};
use forge_parser::ast::Expr;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{ImplicitConversion, ParamType, QualType, Type};

use super::helpers::{int, q, ti};

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

fn float_lit(v: f64, id: u32) -> Expr {
    Expr::FloatLiteral {
        value: v,
        suffix: FloatSuffix::F,
        span: S,
        node_id: NodeId(id),
    }
}

fn call(callee: Expr, args: Vec<Expr>, id: u32) -> Expr {
    Expr::FunctionCall {
        callee: Box::new(callee),
        args,
        span: S,
        node_id: NodeId(id),
    }
}

fn declare_fn(table: &mut SymbolTable, name: &str, ty: QualType, ctx: &mut SemaContext) {
    let sym = Symbol {
        id: 0,
        name: name.to_string(),
        ty,
        kind: SymbolKind::Function,
        storage: StorageClass::None,
        linkage: Linkage::External,
        span: S,
        is_defined: false,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
    };
    table.declare(sym, ctx).expect("declare must succeed");
}

fn make_fn(
    return_ty: QualType,
    params: Vec<QualType>,
    is_variadic: bool,
    is_prototype: bool,
) -> QualType {
    q(Type::Function {
        return_type: Box::new(return_ty),
        params: params
            .into_iter()
            .map(|ty| ParamType {
                name: None,
                ty,
                has_static_size: false,
            })
            .collect(),
        is_variadic,
        is_prototype,
    })
}

#[test]
fn prototyped_call_with_correct_arity_ok() {
    // int f(int, int);  f(1, 2);
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_fn(
        &mut table,
        "f",
        make_fn(q(int()), vec![q(int()), q(int())], false, true),
        &mut ctx,
    );
    let e = call(ident("f", 1), vec![int_lit(1, 2), int_lit(2, 3)], 4);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, int());
}

#[test]
fn prototyped_call_wrong_arity_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_fn(
        &mut table,
        "f",
        make_fn(q(int()), vec![q(int()), q(int())], false, true),
        &mut ctx,
    );
    let e = call(ident("f", 1), vec![int_lit(1, 2)], 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("wrong number"));
}

#[test]
fn variadic_call_passes_float_as_double() {
    // int printf_like(const char *, ...);  printf_like("%f", 1.5f);
    // The 1.5f argument is a variadic slot and must be promoted to
    // double per C17 §6.5.2.2p7.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let mut const_char = q(Type::Char {
        signedness: crate::types::Signedness::Plain,
    });
    const_char.is_const = true;
    let ptr_to_const_char = q(Type::Pointer {
        pointee: Box::new(const_char),
    });
    declare_fn(
        &mut table,
        "p",
        make_fn(q(int()), vec![ptr_to_const_char], true, true),
        &mut ctx,
    );
    let e = call(
        ident("p", 1),
        vec![
            Expr::StringLiteral {
                value: "%f".to_string(),
                prefix: forge_lexer::StringPrefix::None,
                span: S,
                node_id: NodeId(2),
            },
            float_lit(1.5, 3),
        ],
        4,
    );
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(
        ctx.implicit_convs.get(&3),
        Some(&ImplicitConversion::FloatConversion { to: Type::Double })
    );
}

#[test]
fn unprototyped_call_is_warning_not_error() {
    // int f();  f(1, 2);
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_fn(
        &mut table,
        "f",
        make_fn(q(int()), Vec::new(), false, false),
        &mut ctx,
    );
    let e = call(ident("f", 1), vec![int_lit(1, 2), int_lit(2, 3)], 4);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert!(ctx
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Warning));
    assert_eq!(qt.ty, int());
}

#[test]
fn call_through_function_pointer_ok() {
    // int (*fp)(int);  fp(5);
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let fn_ty = make_fn(q(int()), vec![q(int())], false, true);
    let fp_ty = q(Type::Pointer {
        pointee: Box::new(fn_ty),
    });
    let sym = Symbol {
        id: 0,
        name: "fp".to_string(),
        ty: fp_ty,
        kind: SymbolKind::Variable,
        storage: StorageClass::None,
        linkage: Linkage::None,
        span: S,
        is_defined: true,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
    };
    table.declare(sym, &mut ctx).expect("declare must succeed");
    let e = call(ident("fp", 1), vec![int_lit(5, 2)], 3);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
    assert_eq!(qt.ty, int());
}

#[test]
fn call_non_function_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let sym = Symbol {
        id: 0,
        name: "x".to_string(),
        ty: q(int()),
        kind: SymbolKind::Variable,
        storage: StorageClass::None,
        linkage: Linkage::None,
        span: S,
        is_defined: true,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
    };
    table.declare(sym, &mut ctx).expect("declare must succeed");
    let e = call(ident("x", 1), Vec::new(), 2);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
}

#[test]
fn return_type_has_qualifiers_stripped() {
    // `const int f(void);` — calling f() yields plain int (non-const).
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let mut ret = q(int());
    ret.is_const = true;
    declare_fn(
        &mut table,
        "f",
        make_fn(ret, Vec::new(), false, true),
        &mut ctx,
    );
    let e = call(ident("f", 1), Vec::new(), 2);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, int());
    assert!(!qt.is_const);
}

#[test]
fn prototyped_argument_type_mismatch_is_error() {
    use super::helpers::ptr_to;
    // int f(int *);  f(3.14); — float not assignable to int *.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_fn(
        &mut table,
        "f",
        make_fn(q(int()), vec![q(ptr_to(q(int())))], false, true),
        &mut ctx,
    );
    let e = call(ident("f", 1), vec![float_lit(1.5, 2)], 3);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
}

#[test]
fn variadic_promotes_char_argument_to_int() {
    // int f(int, ...);  f(0, 'a');
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_fn(
        &mut table,
        "f",
        make_fn(q(int()), vec![q(int())], true, true),
        &mut ctx,
    );
    let e = call(
        ident("f", 1),
        vec![
            int_lit(0, 2),
            Expr::CharLiteral {
                value: u32::from(b'a'),
                prefix: forge_lexer::CharPrefix::None,
                span: S,
                node_id: NodeId(3),
            },
        ],
        4,
    );
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "{:?}", ctx.diagnostics);
}
