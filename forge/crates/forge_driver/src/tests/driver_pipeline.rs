//! End-to-end driver pipeline tests.
//!
//! Exercises [`crate::compile`] across the three [`crate::CompileStage`]
//! values.  The contract these tests pin down:
//!
//! * `Sema` (the default) runs the full lex → preprocess → parse → sema
//!   pipeline and surfaces every phase's diagnostics.  Type-side
//!   tables on the returned `SemaContext` are populated and the
//!   `SymbolTable` is non-empty.
//! * `Parse` stops after the parser — no sema errors fire, even on
//!   sources that would have failed sema (e.g. undeclared identifiers).
//! * Type annotations land on expression node-ids, mirroring what the
//!   CLI's `--dump-types` flag reads back.

use forge_diagnostics::Severity;
use forge_parser::ast::{Expr, ExternalDeclaration, Stmt};

use crate::{compile, CompileOptions, CompileStage};

fn sema_opts() -> CompileOptions {
    CompileOptions::default()
}

fn parse_opts() -> CompileOptions {
    CompileOptions {
        stage: CompileStage::Parse,
        ..CompileOptions::default()
    }
}

// =========================================================================
// Section 1 — valid input exits cleanly at Sema stage.
// =========================================================================

#[test]
fn sema_accepts_well_formed_source() {
    let out = compile("valid.c", "int main(void) { return 0; }", &sema_opts());
    assert!(
        !out.has_errors(),
        "expected no errors, got: {:?}",
        out.diagnostics
    );
    assert!(
        out.sema.is_some(),
        "sema side table must be populated at Sema stage"
    );
    assert!(
        out.symbol_table.is_some(),
        "symbol table must be populated at Sema stage"
    );
    let table = out.symbol_table.as_ref().expect("symbol_table");
    assert!(
        table.lookup("main").is_some(),
        "main must be in the symbol table"
    );
}

#[test]
fn sema_accepts_library_headers_worth_of_decls() {
    // A larger fragment that mimics the shape of a typical standard
    // header entry: typedefs, struct tags, function prototypes.
    let src = r#"
        typedef unsigned long size_t;
        struct S { int a; int b; };
        int strlen_like(const char *);
        int main(void) { return 0; }
    "#;
    let out = compile("library.c", src, &sema_opts());
    assert!(
        !out.has_errors(),
        "expected no errors, got: {:?}",
        out.diagnostics
    );
}

// =========================================================================
// Section 2 — ill-typed input emits error diagnostics.
// =========================================================================

#[test]
fn sema_reports_undeclared_identifier() {
    let out = compile(
        "bad.c",
        "int main(void) { return undeclared; }",
        &sema_opts(),
    );
    assert!(
        out.has_errors(),
        "undeclared identifier must reach sema and error: {:?}",
        out.diagnostics
    );
    assert!(
        out.diagnostics
            .iter()
            .any(|d| matches!(d.severity, Severity::Error)),
        "expected at least one error-severity diagnostic"
    );
}

#[test]
fn sema_reports_value_returned_from_void_function() {
    // Returning a value from a void function is a hard error per
    // [`forge_sema`]'s return-statement contract.
    let out = compile(
        "bad.c",
        r#"
            void f(void) { return 42; }
            int main(void) { return 0; }
        "#,
        &sema_opts(),
    );
    assert!(
        out.has_errors(),
        "expected at least one error, got: {:?}",
        out.diagnostics
    );
}

// =========================================================================
// Section 3 — Parse stage does not run sema.
// =========================================================================

#[test]
fn parse_stage_skips_sema_entirely() {
    // Undeclared identifiers only fail in sema; at Parse stage the
    // driver stops earlier and must not surface that diagnostic.
    let out = compile(
        "ok_to_parse.c",
        "int main(void) { return undeclared; }",
        &parse_opts(),
    );
    assert!(
        !out.has_errors(),
        "sema should not run at Parse stage: {:?}",
        out.diagnostics
    );
    assert!(
        out.sema.is_none(),
        "Parse stage must not populate the sema side tables"
    );
    assert!(
        out.symbol_table.is_none(),
        "Parse stage must not populate the symbol table"
    );
}

#[test]
fn parse_stage_still_yields_ast() {
    let out = compile(
        "parse_only.c",
        "int main(void) { return 0; }",
        &parse_opts(),
    );
    let ast = out.ast.as_ref().expect("parser always yields an AST");
    assert!(
        !ast.declarations.is_empty(),
        "AST should contain the main function"
    );
}

// =========================================================================
// Section 4 — --dump-types surface: sema records expression types.
// =========================================================================

#[test]
fn sema_annotates_expression_types() {
    // A `return x + 1;` records types on the binary-op and both operands.
    // The CLI's --dump-types flag walks these node-id entries; we assert
    // the side table has content for the return-expression path.
    let src = "int main(void) { int x = 3; return x + 1; }";
    let out = compile("types.c", src, &sema_opts());
    assert!(!out.has_errors(), "{:?}", out.diagnostics);

    let sema = out.sema.as_ref().expect("sema present at Sema stage");
    let ast = out.ast.as_ref().expect("ast present at Sema stage");

    // Find the return statement's expression node id.
    let mut return_expr_node = None;
    for ext in &ast.declarations {
        if let ExternalDeclaration::FunctionDef(fd) = ext {
            for item in &fd.body.items {
                if let forge_parser::ast::BlockItem::Statement(Stmt::Return {
                    value: Some(expr),
                    ..
                }) = item
                {
                    return_expr_node = Some(expr_node_id(expr));
                }
            }
        }
    }
    let node = return_expr_node.expect("return statement must be present");
    assert!(
        sema.get_type(node).is_some(),
        "return expression node {node:?} must have a type recorded"
    );
}

fn expr_node_id(e: &Expr) -> forge_parser::node_id::NodeId {
    match e {
        Expr::IntLiteral { node_id, .. }
        | Expr::FloatLiteral { node_id, .. }
        | Expr::CharLiteral { node_id, .. }
        | Expr::StringLiteral { node_id, .. }
        | Expr::Ident { node_id, .. }
        | Expr::MemberAccess { node_id, .. }
        | Expr::ArraySubscript { node_id, .. }
        | Expr::SizeofExpr { node_id, .. }
        | Expr::SizeofType { node_id, .. }
        | Expr::AlignofType { node_id, .. }
        | Expr::UnaryOp { node_id, .. }
        | Expr::PostfixOp { node_id, .. }
        | Expr::BinaryOp { node_id, .. }
        | Expr::Assignment { node_id, .. }
        | Expr::Conditional { node_id, .. }
        | Expr::FunctionCall { node_id, .. }
        | Expr::Cast { node_id, .. }
        | Expr::CompoundLiteral { node_id, .. }
        | Expr::GenericSelection { node_id, .. }
        | Expr::Comma { node_id, .. }
        | Expr::BuiltinOffsetof { node_id, .. }
        | Expr::BuiltinTypesCompatibleP { node_id, .. } => *node_id,
    }
}

// =========================================================================
// Section 5 — Preprocess stage terminates before parse.
// =========================================================================

#[test]
fn preprocess_stage_skips_parse_and_sema() {
    let options = CompileOptions {
        stage: CompileStage::Preprocess,
        ..CompileOptions::default()
    };
    let out = compile("pp.c", "int main(void) { return undeclared; }", &options);
    assert!(out.ast.is_none(), "preprocess stage must not run parse");
    assert!(out.sema.is_none(), "preprocess stage must not run sema");
    assert!(
        out.symbol_table.is_none(),
        "preprocess stage must not run sema"
    );
}

// =========================================================================
// Section 6 — GNU __extension__ is transparent end-to-end.
// =========================================================================

#[test]
fn extension_marker_is_transparent_through_the_pipeline() {
    // `__extension__` must be a no-op at every stage: preprocess,
    // parse, and sema all pass.  Migrated from the inline GNU tests
    // to keep the end-to-end story in one place.
    let out = compile(
        "ext.c",
        r#"
            __extension__ typedef int my_int;
            int main(void) {
                __extension__ my_int x = 0;
                return x;
            }
        "#,
        &sema_opts(),
    );
    assert!(!out.has_errors(), "{:?}", out.diagnostics);
}
