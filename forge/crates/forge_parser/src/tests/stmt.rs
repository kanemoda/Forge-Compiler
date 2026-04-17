//! Tests for Prompt 3.5 — statement parsing.

use crate::ast::*;

use super::helpers::{parse_stmt, parse_tu};

// =========================================================================
// Helpers
// =========================================================================

/// Unwrap `Stmt::Compound`.
fn as_compound(s: &Stmt) -> &CompoundStmt {
    match s {
        Stmt::Compound(c) => c,
        other => panic!("expected Stmt::Compound, got {other:?}"),
    }
}

/// Unwrap `BlockItem::Statement`.
fn as_stmt(item: &BlockItem) -> &Stmt {
    match item {
        BlockItem::Statement(s) => s,
        other => panic!("expected BlockItem::Statement, got {other:?}"),
    }
}

/// Unwrap `BlockItem::Declaration`.
fn as_decl(item: &BlockItem) -> &Declaration {
    match item {
        BlockItem::Declaration(d) => d,
        other => panic!("expected BlockItem::Declaration, got {other:?}"),
    }
}

/// Unwrap `Stmt::Expr`.
fn as_expr_stmt(s: &Stmt) -> Option<&Expr> {
    match s {
        Stmt::Expr { expr, .. } => expr.as_deref(),
        other => panic!("expected Stmt::Expr, got {other:?}"),
    }
}

/// Unwrap an `Expr::IntLiteral`.
fn int_lit(e: &Expr) -> u64 {
    match e {
        Expr::IntLiteral { value, .. } => *value,
        other => panic!("expected IntLiteral, got {other:?}"),
    }
}

/// Unwrap an `Expr::Ident`.
fn ident(e: &Expr) -> &str {
    match e {
        Expr::Ident { name, .. } => name.as_str(),
        other => panic!("expected Ident, got {other:?}"),
    }
}

/// The body of a single-function translation unit.
fn single_fn_body(src: &str) -> CompoundStmt {
    let tu = parse_tu(src);
    assert_eq!(tu.declarations.len(), 1);
    match &tu.declarations[0] {
        ExternalDeclaration::FunctionDef(f) => f.body.clone(),
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

// =========================================================================
// Simple statements
// =========================================================================

#[test]
fn empty_statement() {
    let s = parse_stmt(";");
    assert!(as_expr_stmt(&s).is_none());
}

#[test]
fn expression_statement() {
    let s = parse_stmt("x + 1;");
    assert!(as_expr_stmt(&s).is_some());
}

#[test]
fn compound_empty() {
    let s = parse_stmt("{}");
    let c = as_compound(&s);
    assert!(c.items.is_empty());
}

#[test]
fn compound_mixed_items() {
    // Declarations and statements may freely interleave (C99+).
    let s = parse_stmt("{ int x = 1; x = x + 2; int y; }");
    let c = as_compound(&s);
    assert_eq!(c.items.len(), 3);
    let _ = as_decl(&c.items[0]);
    let _ = as_stmt(&c.items[1]);
    let _ = as_decl(&c.items[2]);
}

// =========================================================================
// Jump statements
// =========================================================================

#[test]
fn return_without_value() {
    let s = parse_stmt("return;");
    match s {
        Stmt::Return { value, .. } => assert!(value.is_none()),
        other => panic!("expected Return, got {other:?}"),
    }
}

#[test]
fn return_with_value() {
    let s = parse_stmt("return 42;");
    match s {
        Stmt::Return { value: Some(e), .. } => assert_eq!(int_lit(&e), 42),
        other => panic!("expected Return 42, got {other:?}"),
    }
}

#[test]
fn break_statement() {
    assert!(matches!(parse_stmt("break;"), Stmt::Break { .. }));
}

#[test]
fn continue_statement() {
    assert!(matches!(parse_stmt("continue;"), Stmt::Continue { .. }));
}

#[test]
fn goto_statement() {
    match parse_stmt("goto end;") {
        Stmt::Goto { label, .. } => assert_eq!(label, "end"),
        other => panic!("expected Goto, got {other:?}"),
    }
}

#[test]
fn label_statement() {
    match parse_stmt("end: return 0;") {
        Stmt::Label { name, stmt, .. } => {
            assert_eq!(name, "end");
            assert!(matches!(*stmt, Stmt::Return { .. }));
        }
        other => panic!("expected Label, got {other:?}"),
    }
}

// =========================================================================
// if / else
// =========================================================================

#[test]
fn if_no_else() {
    match parse_stmt("if (x) return 1;") {
        Stmt::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            assert_eq!(ident(&condition), "x");
            assert!(matches!(*then_branch, Stmt::Return { .. }));
            assert!(else_branch.is_none());
        }
        other => panic!("expected If, got {other:?}"),
    }
}

#[test]
fn if_with_else() {
    match parse_stmt("if (x) return 1; else return 2;") {
        Stmt::If {
            then_branch,
            else_branch: Some(else_branch),
            ..
        } => {
            assert!(matches!(*then_branch, Stmt::Return { .. }));
            assert!(matches!(*else_branch, Stmt::Return { .. }));
        }
        other => panic!("expected If/else, got {other:?}"),
    }
}

/// Classic dangling-else disambiguation: the else binds to the *nearest*
/// if.  Given `if (a) if (b) X; else Y;`, Y is the else of the inner if.
#[test]
fn dangling_else_binds_to_nearest_if() {
    match parse_stmt("if (a) if (b) x(); else y();") {
        Stmt::If {
            then_branch,
            else_branch,
            ..
        } => {
            // Outer if has no else.
            assert!(else_branch.is_none(), "outer if must not claim the else");
            // The inner if owns the else.
            match *then_branch {
                Stmt::If {
                    else_branch: Some(_),
                    ..
                } => {}
                other => panic!("expected inner if-with-else, got {other:?}"),
            }
        }
        other => panic!("expected If, got {other:?}"),
    }
}

// =========================================================================
// Loops
// =========================================================================

#[test]
fn while_loop() {
    match parse_stmt("while (i < 10) i = i + 1;") {
        Stmt::While {
            condition, body, ..
        } => {
            assert!(matches!(*condition, Expr::BinaryOp { .. }));
            assert!(matches!(*body, Stmt::Expr { .. }));
        }
        other => panic!("expected While, got {other:?}"),
    }
}

#[test]
fn do_while_loop() {
    match parse_stmt("do { x = x + 1; } while (x < 10);") {
        Stmt::DoWhile {
            body, condition, ..
        } => {
            assert!(matches!(*body, Stmt::Compound(_)));
            assert!(matches!(*condition, Expr::BinaryOp { .. }));
        }
        other => panic!("expected DoWhile, got {other:?}"),
    }
}

#[test]
fn for_with_declaration_init() {
    match parse_stmt("for (int i = 0; i < 10; i = i + 1) i;") {
        Stmt::For {
            init: Some(ForInit::Declaration(_)),
            condition: Some(_),
            update: Some(_),
            body,
            ..
        } => {
            assert!(matches!(*body, Stmt::Expr { .. }));
        }
        other => panic!("expected For with decl init, got {other:?}"),
    }
}

#[test]
fn for_with_expression_init() {
    match parse_stmt("for (i = 0; i < 10; i = i + 1) ;") {
        Stmt::For {
            init: Some(ForInit::Expr(_)),
            condition: Some(_),
            update: Some(_),
            body,
            ..
        } => {
            // Empty body is a (None) Stmt::Expr.
            assert!(as_expr_stmt(&body).is_none());
        }
        other => panic!("expected For with expr init, got {other:?}"),
    }
}

#[test]
fn for_with_empty_clauses() {
    match parse_stmt("for (;;) break;") {
        Stmt::For {
            init: None,
            condition: None,
            update: None,
            body,
            ..
        } => {
            assert!(matches!(*body, Stmt::Break { .. }));
        }
        other => panic!("expected empty-for, got {other:?}"),
    }
}

// =========================================================================
// switch / case / default
// =========================================================================

#[test]
fn switch_with_cases() {
    let s = parse_stmt("switch (x) { case 1: return 1; case 2: return 2; default: return 0; }");
    let Stmt::Switch { expr, body, .. } = &s else {
        panic!("expected Switch, got {s:?}");
    };
    assert_eq!(ident(expr), "x");
    let compound = as_compound(body);
    assert_eq!(compound.items.len(), 3);

    // All three items should be statements — case / case / default.
    assert!(matches!(as_stmt(&compound.items[0]), Stmt::Case { .. }));
    assert!(matches!(as_stmt(&compound.items[1]), Stmt::Case { .. }));
    assert!(matches!(as_stmt(&compound.items[2]), Stmt::Default { .. }));
}

#[test]
fn case_captures_constant_and_body() {
    let s = parse_stmt("case 7: return 7;");
    match s {
        Stmt::Case { value, body, .. } => {
            assert_eq!(int_lit(&value), 7);
            assert!(matches!(*body, Stmt::Return { .. }));
        }
        other => panic!("expected Case, got {other:?}"),
    }
}

// =========================================================================
// Scoping — typedef visibility inside compound / for
// =========================================================================

#[test]
fn compound_scopes_dont_leak_typedefs() {
    // Declaring a typedef inside a block must not affect later top-level
    // parsing — if it leaked, the second `T` would be lexed as a
    // typedef-name and the parse of `T * x = 0;` would succeed as a
    // declaration instead of the intended expression-statement.  We
    // check this indirectly: parse a complete translation unit where
    // the inner typedef is local.
    let src = "\
        void f(void) { typedef int T; T x = 0; } \
        int T = 1; \
    ";
    let tu = parse_tu(src);
    assert_eq!(tu.declarations.len(), 2);
}

#[test]
fn for_loop_init_typedef_scope() {
    // `for (typedef int T; 0; ) T x;` — T should be visible in the body
    // but not leak out.  We assert the body parses T as a typedef.
    let src = "\
        void f(void) { for (typedef int T; 0; ) { T x; } int T = 1; } \
    ";
    let tu = parse_tu(src);
    let ExternalDeclaration::FunctionDef(f) = &tu.declarations[0] else {
        panic!("expected FunctionDef");
    };
    // The outer compound has two items: the for loop and the `int T = 1;`.
    assert_eq!(f.body.items.len(), 2);
    let _ = as_stmt(&f.body.items[0]); // the for
    let _ = as_decl(&f.body.items[1]); // int T = 1 (T as identifier)
}

// =========================================================================
// _Static_assert at block scope
// =========================================================================

#[test]
fn static_assert_in_compound() {
    let src = "void f(void) { _Static_assert(1, \"ok\"); int x; }";
    let body = single_fn_body(src);
    assert_eq!(body.items.len(), 2);
    match &body.items[0] {
        BlockItem::StaticAssert(sa) => assert_eq!(sa.message.as_deref(), Some("ok")),
        other => panic!("expected StaticAssert, got {other:?}"),
    }
}

#[test]
fn static_assert_c23_no_message() {
    let src = "void f(void) { _Static_assert(1); }";
    let body = single_fn_body(src);
    assert_eq!(body.items.len(), 1);
    match &body.items[0] {
        BlockItem::StaticAssert(sa) => assert!(sa.message.is_none()),
        other => panic!("expected StaticAssert, got {other:?}"),
    }
}
