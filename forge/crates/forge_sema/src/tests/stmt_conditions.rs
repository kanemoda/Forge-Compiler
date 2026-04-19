//! Tests for conditional and loop-statement type checking.
//!
//! Covers the scalar-condition requirement on `if`, `while`, `do`, and
//! `for`, plus the `if (x = 5)` typo warning.

use crate::scope::{ScopeKind, SymbolTable};
use crate::stmt::analyze_stmt;
use crate::{context::SemaContext, types::QualType};

use super::helpers::*;

fn drive(stmt_builder: impl FnOnce() -> forge_parser::ast::Stmt) -> SemaContext {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table.push_scope(ScopeKind::Function);
    let mut fnc = fn_ctx(QualType::unqualified(int()));
    let s = stmt_builder();
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);
    ctx
}

// ---------------------------------------------------------------------
// if-statement
// ---------------------------------------------------------------------

#[test]
fn if_with_integer_condition_accepts() {
    let ctx = drive(|| h_if(h_int_lit(1), h_empty_stmt(), None));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn if_with_assignment_condition_emits_typo_warning() {
    // `if (x = 5)` — assignment, not comparison.  Declare `x` up front
    // so the expression type-checks cleanly; the warning itself does not
    // depend on the outcome of type checking.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table.push_scope(ScopeKind::Function);
    let d = h_declaration(h_int_specs(), vec![h_init_decl(h_ident_decl("x"), None)]);
    crate::declare::analyze_declaration(&d, &mut table, &ti(), &mut ctx);

    let mut fnc = fn_ctx(QualType::unqualified(int()));
    let s = h_if(
        h_assign(h_ident_expr("x"), h_int_lit(5)),
        h_empty_stmt(),
        None,
    );
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);

    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("assignment used as condition")),
        "expected assignment-in-condition warning, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn if_condition_must_be_scalar() {
    // Build a struct-valued condition — concretely a compound literal of
    // an empty struct.  Easiest way: declare `struct S { int x; } s;` and
    // use `s` as the controlling expression.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table.push_scope(ScopeKind::Function);

    // struct S { int x; } s;
    let sdef = forge_parser::ast::StructDef {
        kind: forge_parser::ast::StructOrUnion::Struct,
        name: Some("S".into()),
        members: Some(vec![forge_parser::ast::StructMember::Field(
            forge_parser::ast::StructField {
                specifiers: h_int_specs(),
                declarators: vec![forge_parser::ast::StructFieldDeclarator {
                    declarator: Some(h_ident_decl("x")),
                    bit_width: None,
                    span: HS,
                }],
                span: HS,
                node_id: HN,
            },
        )]),
        attributes: Vec::new(),
        span: HS,
    };
    let d = h_declaration(
        h_specs(vec![forge_parser::ast::TypeSpecifierToken::Struct(sdef)]),
        vec![h_init_decl(h_ident_decl("s"), None)],
    );
    crate::declare::analyze_declaration(&d, &mut table, &ti(), &mut ctx);

    let mut fnc = fn_ctx(QualType::unqualified(int()));
    let s = h_if(h_ident_expr("s"), h_empty_stmt(), None);
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);

    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("'if' condition must have scalar type")),
        "expected scalar-condition error, got {:?}",
        ctx.diagnostics
    );
}

// ---------------------------------------------------------------------
// while / do-while
// ---------------------------------------------------------------------

#[test]
fn while_with_integer_condition_accepts() {
    let ctx = drive(|| h_while(h_int_lit(0), h_empty_stmt()));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn do_while_with_integer_condition_accepts() {
    let ctx = drive(|| h_do_while(h_empty_stmt(), h_int_lit(0)));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

// ---------------------------------------------------------------------
// for
// ---------------------------------------------------------------------

#[test]
fn for_all_clauses_optional_is_infinite_loop() {
    let ctx = drive(|| h_for(None, None, None, h_empty_stmt()));
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn for_init_declaration_is_scoped_to_loop() {
    // `for (int i = 0; i; i) ;` — `i` is only visible inside the loop.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table.push_scope(ScopeKind::Function);
    let mut fnc = fn_ctx(QualType::unqualified(int()));

    let init = forge_parser::ast::ForInit::Declaration(h_declaration(
        h_int_specs(),
        vec![h_init_decl(h_ident_decl("i"), Some(h_expr_init(0)))],
    ));
    let s = h_for(
        Some(init),
        Some(h_ident_expr("i")),
        Some(h_ident_expr("i")),
        h_empty_stmt(),
    );
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    assert!(
        table.lookup("i").is_none(),
        "`i` must not leak out of the for-loop scope"
    );
}

#[test]
fn for_condition_non_scalar_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table.push_scope(ScopeKind::Function);

    // Introduce a struct-valued `s` at function scope.
    let sdef = forge_parser::ast::StructDef {
        kind: forge_parser::ast::StructOrUnion::Struct,
        name: Some("S".into()),
        members: Some(vec![forge_parser::ast::StructMember::Field(
            forge_parser::ast::StructField {
                specifiers: h_int_specs(),
                declarators: vec![forge_parser::ast::StructFieldDeclarator {
                    declarator: Some(h_ident_decl("x")),
                    bit_width: None,
                    span: HS,
                }],
                span: HS,
                node_id: HN,
            },
        )]),
        attributes: Vec::new(),
        span: HS,
    };
    let d = h_declaration(
        h_specs(vec![forge_parser::ast::TypeSpecifierToken::Struct(sdef)]),
        vec![h_init_decl(h_ident_decl("s"), None)],
    );
    crate::declare::analyze_declaration(&d, &mut table, &ti(), &mut ctx);

    let mut fnc = fn_ctx(QualType::unqualified(int()));
    let s = h_for(None, Some(h_ident_expr("s")), None, h_empty_stmt());
    analyze_stmt(&s, &mut fnc, &mut table, &ti(), &mut ctx);

    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("'for' condition must have scalar type")),
        "expected scalar-condition error for non-scalar for-cond, got {:?}",
        ctx.diagnostics
    );
}
