//! Shared test helpers for the parser crate.
//!
//! NOT a test file — no `#[test]` functions here.

use forge_diagnostics::Diagnostic;
use forge_lexer::{Lexer, Token};

use crate::ast::{Declaration, Expr, Stmt, TranslationUnit, TypeName};
use crate::parser::Parser;

/// Lex a C expression string, wrap it so the lexer is happy, then parse
/// it as an expression via the Pratt parser.
///
/// Returns the parsed `Expr`.  Panics on lex errors.
pub fn parse_expr(src: &str) -> Expr {
    let tokens = lex(src);
    let mut parser = Parser::new(tokens);
    parser.parse_expr()
}

/// Parse a single top-level declaration.  Asserts the parser reaches
/// EOF immediately after and emitted no error-severity diagnostics.
pub fn parse_decl(src: &str) -> Declaration {
    let tokens = lex(src);
    let mut parser = Parser::new(tokens);
    let decl = parser.parse_declaration();
    assert!(parser.at_eof(), "unexpected tokens after declaration");
    assert_no_errors(parser.take_diagnostics(), src);
    decl
}

/// Parse one or more top-level declarations in sequence, stopping at
/// EOF.  Asserts no error-severity diagnostics were produced.
pub fn parse_decls(src: &str) -> Vec<Declaration> {
    let tokens = lex(src);
    let mut parser = Parser::new(tokens);
    let mut decls = Vec::new();
    while !parser.at_eof() {
        decls.push(parser.parse_declaration());
    }
    assert_no_errors(parser.take_diagnostics(), src);
    decls
}

/// Parse a type-name (as would appear in a cast).  Returns `None` when
/// the input is not a type-name.
pub fn parse_type_name(src: &str) -> Option<TypeName> {
    let tokens = lex(src);
    let mut parser = Parser::new(tokens);
    parser.parse_type_name()
}

/// Parse a single statement.  Asserts the parser reaches EOF afterwards
/// and produced no error-severity diagnostics.
pub fn parse_stmt(src: &str) -> Stmt {
    let tokens = lex(src);
    let mut parser = Parser::new(tokens);
    let stmt = parser.parse_statement();
    assert!(parser.at_eof(), "unexpected tokens after statement");
    assert_no_errors(parser.take_diagnostics(), src);
    stmt
}

/// Parse a full translation unit via `Parser::parse`.  Asserts no
/// error-severity diagnostics were produced.
pub fn parse_tu(src: &str) -> TranslationUnit {
    let tokens = lex(src);
    let (tu, diags) = Parser::parse(tokens);
    assert_no_errors(diags, src);
    tu
}

/// Parse a translation unit without asserting errors, so tests can
/// inspect the resulting diagnostics.
pub fn parse_tu_with_diagnostics(src: &str) -> (TranslationUnit, Vec<Diagnostic>) {
    Parser::parse(lex(src))
}

/// Lex source text into a token stream.
pub fn lex(src: &str) -> Vec<Token> {
    Lexer::new(src).tokenize()
}

fn assert_no_errors(diagnostics: Vec<Diagnostic>, src: &str) {
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.severity == forge_diagnostics::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "unexpected errors parsing {src:?}: {errors:#?}"
    );
}
