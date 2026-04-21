//! Shared helpers used by every test file in this submodule.
//!
//! The lexer always emits a trailing [`TokenKind::Eof`]; every helper here
//! strips it off so tests only need to look at the substantive tokens.

pub use forge_diagnostics::{Diagnostic, FileId};

use crate::{CharPrefix, FloatSuffix, IntSuffix, Lexer, StringPrefix, Token, TokenKind};

/// Tokenize `src` and return the non-`Eof` tokens.
///
/// Panics if the lexer does not end with `Eof` (which it always does).
pub fn lex(src: &str) -> Vec<Token> {
    lex_with_diags(src).0
}

/// Tokenize `src` and return the non-`Eof` tokens along with every
/// diagnostic the lexer produced.
pub fn lex_with_diags(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lx = Lexer::new(src, FileId::PRIMARY);
    let mut toks = lx.tokenize();
    let last = toks
        .pop()
        .expect("tokenize must always produce at least Eof");
    assert!(
        matches!(last.kind, TokenKind::Eof),
        "last token must be Eof"
    );
    let diags = lx.take_diagnostics();
    (toks, diags)
}

/// Tokenize `src` and return only the token kinds (trailing `Eof` removed).
pub fn kinds(src: &str) -> Vec<TokenKind> {
    lex(src).into_iter().map(|t| t.kind).collect()
}

/// Tokenize `src` and assert the result is exactly one token with no
/// diagnostics; return that token's kind.
pub fn single_clean(src: &str) -> TokenKind {
    let (toks, diags) = lex_with_diags(src);
    assert!(
        diags.is_empty(),
        "unexpected diagnostics for `{src}`: {diags:?}"
    );
    assert_eq!(
        toks.len(),
        1,
        "expected one token for `{src}`, got {toks:?}"
    );
    toks[0].kind.clone()
}

/// Extract an [`IntegerLiteral`](TokenKind::IntegerLiteral) value + suffix,
/// panicking with a useful message if the token is some other kind.
pub fn as_int(k: &TokenKind) -> (u64, IntSuffix) {
    match k {
        TokenKind::IntegerLiteral { value, suffix } => (*value, *suffix),
        other => panic!("expected IntegerLiteral, got {other:?}"),
    }
}

/// Extract a [`FloatLiteral`](TokenKind::FloatLiteral) value + suffix.
pub fn as_float(k: &TokenKind) -> (f64, FloatSuffix) {
    match k {
        TokenKind::FloatLiteral { value, suffix } => (*value, *suffix),
        other => panic!("expected FloatLiteral, got {other:?}"),
    }
}

/// Extract a [`CharLiteral`](TokenKind::CharLiteral) value + prefix.
pub fn as_char(k: &TokenKind) -> (u32, CharPrefix) {
    match k {
        TokenKind::CharLiteral { value, prefix } => (*value, *prefix),
        other => panic!("expected CharLiteral, got {other:?}"),
    }
}

/// Extract a [`StringLiteral`](TokenKind::StringLiteral) value + prefix.
pub fn as_string(k: &TokenKind) -> (String, StringPrefix) {
    match k {
        TokenKind::StringLiteral { value, prefix } => (value.clone(), *prefix),
        other => panic!("expected StringLiteral, got {other:?}"),
    }
}
