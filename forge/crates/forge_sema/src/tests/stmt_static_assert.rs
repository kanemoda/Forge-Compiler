//! Tests for block-scope `_Static_assert`.
//!
//! `_Static_assert` inside a function body is handled by the block-item
//! walker so its diagnostics appear just like any other statement error.

use forge_parser::ast::{BlockItem, StaticAssert};

use crate::stmt::analyze_function_def;
use crate::{context::SemaContext, scope::SymbolTable};

use super::helpers::*;

fn run(func: forge_parser::ast::FunctionDef) -> SemaContext {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    analyze_function_def(&func, &mut table, &ti(), &mut ctx);
    ctx
}

#[test]
fn block_scope_static_assert_true_is_silent() {
    let sa = StaticAssert {
        condition: Box::new(h_int_lit(1)),
        message: Some("should pass".into()),
        span: HS,
    };
    let body = vec![BlockItem::StaticAssert(sa)];
    let ctx = run(h_fn_void_void("f", body));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn block_scope_static_assert_false_emits_error_with_message() {
    let sa = StaticAssert {
        condition: Box::new(h_int_lit(0)),
        message: Some("kaboom".into()),
        span: HS,
    };
    let body = vec![BlockItem::StaticAssert(sa)];
    let ctx = run(h_fn_void_void("f", body));
    assert!(
        ctx.diagnostics.iter().any(|d| d.message.contains("kaboom")),
        "expected user-supplied failure message, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn block_scope_static_assert_false_without_message_uses_default() {
    let sa = StaticAssert {
        condition: Box::new(h_int_lit(0)),
        message: None,
        span: HS,
    };
    let body = vec![BlockItem::StaticAssert(sa)];
    let ctx = run(h_fn_void_void("f", body));
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("static assertion failed")),
        "expected default failure message, got {:?}",
        ctx.diagnostics
    );
}
