//! Shared test helpers.

use std::fs;
use std::path::PathBuf;

use forge_diagnostics::{Diagnostic, Severity};
use forge_lexer::{Lexer, Token, TokenKind};

use crate::{PreprocessConfig, Preprocessor};

/// The display name the `run` helper hands to the preprocessor when it
/// has no on-disk file.  Matches the private `DEFAULT_INPUT_NAME` used
/// by the production code path.
const DEFAULT_INPUT_NAME: &str = "<input>";

/// Lex `src` into a token stream, keeping the trailing `Eof` sentinel.
pub fn lex(src: &str) -> Vec<Token> {
    Lexer::new(src).tokenize()
}

/// Drive the preprocessor with the given source text, returning the
/// owner so tests can inspect state and collected diagnostics.
pub fn run(src: &str) -> (Preprocessor, Vec<Token>) {
    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_with_source(lex(src), src, DEFAULT_INPUT_NAME);
    (pp, out)
}

/// `true` when `diags` contains no `Severity::Error` entries.
pub fn no_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().all(|d| !matches!(d.severity, Severity::Error))
}

/// Collapse a token stream to just its non-Eof identifier / literal
/// spellings, so comparisons against an expected word list read
/// naturally.
pub fn identifier_names(tokens: &[Token]) -> Vec<String> {
    tokens
        .iter()
        .filter_map(|t| match &t.kind {
            TokenKind::Identifier(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// Return the raw `TokenKind` list for the given stream.
pub fn kinds_of(tokens: &[Token]) -> Vec<TokenKind> {
    tokens.iter().map(|t| t.kind.clone()).collect()
}

/// Expect the first token to be a `StringLiteral` and return its value.
pub fn only_string(out: &[Token]) -> String {
    match &out[0].kind {
        TokenKind::StringLiteral { value, .. } => value.clone(),
        other => panic!("expected StringLiteral, got {other:?}"),
    }
}

/// All non-Eof tokens in the stream.
pub fn non_eof(tokens: &[Token]) -> Vec<&Token> {
    tokens
        .iter()
        .filter(|t| !matches!(t.kind, TokenKind::Eof))
        .collect()
}

/// Extract every integer-literal value in order.
pub fn int_literal_values(tokens: &[Token]) -> Vec<u64> {
    tokens
        .iter()
        .filter_map(|t| match &t.kind {
            TokenKind::IntegerLiteral { value, .. } => Some(*value),
            _ => None,
        })
        .collect()
}

/// Extract every string-literal value in order.
pub fn string_literal_values(tokens: &[Token]) -> Vec<String> {
    tokens
        .iter()
        .filter_map(|t| match &t.kind {
            TokenKind::StringLiteral { value, .. } => Some(value.clone()),
            _ => None,
        })
        .collect()
}

/// Write `body` into `dir/name`, creating missing parents, and return
/// the absolute path.
pub fn write_file(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, body).unwrap();
    path
}

/// All `Severity::Error` entries in `diags`.
pub fn errors_of(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Error))
        .collect()
}

/// All `Severity::Warning` entries in `diags`.
pub fn warnings_of(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Warning))
        .collect()
}

/// All `Severity::Note` entries in `diags`.
pub fn notes_of(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Note))
        .collect()
}

/// Run `src`, then return the first `IntegerLiteral` value in the
/// output — used by `#line` tests that want to read back `__LINE__`.
pub fn line_value_from_macro(src: &str) -> u64 {
    let (_, out) = run(src);
    for t in out {
        if let TokenKind::IntegerLiteral { value, .. } = t.kind {
            return value;
        }
    }
    panic!("no IntegerLiteral in output");
}

/// Small helper trait for brevity at call sites.
pub trait DiagsExt {
    fn is_empty_or_no_errors(&self) -> bool;
}

impl DiagsExt for Vec<Diagnostic> {
    fn is_empty_or_no_errors(&self) -> bool {
        self.iter().all(|d| !matches!(d.severity, Severity::Error))
    }
}
