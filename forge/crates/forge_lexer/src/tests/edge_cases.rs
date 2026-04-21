//! End-of-input behaviour, error recovery, iterator parity, the
//! `lex_fragment` helper, and "no-panic on garbage input" guarantees.

use super::helpers::*;
use crate::{lex_fragment, IntSuffix, Lexer, Span, TokenKind};

// =====================================================================
// Basic shape — empty / non-ASCII
// =====================================================================

#[test]
fn empty_source_emits_only_eof() {
    let toks = Lexer::new("", FileId::PRIMARY).tokenize();
    assert_eq!(toks.len(), 1);
    assert!(matches!(toks[0].kind, TokenKind::Eof));
    assert_eq!(toks[0].span, Span::primary(0, 0));
}

#[test]
fn non_ascii_character_is_unknown_and_span_covers_utf8() {
    // `é` is 0xC3 0xA9 in UTF-8 — two bytes, one code point.
    let toks = lex("é");
    assert_eq!(toks.len(), 1);
    assert_eq!(toks[0].kind, TokenKind::Unknown('é'));
    assert_eq!(toks[0].span, Span::primary(0, 2));
}

// =====================================================================
// Iterator parity
// =====================================================================

#[test]
fn iterator_yields_same_tokens_as_tokenize() {
    let src = "int x; return 0;";
    let via_tokenize = Lexer::new(src, FileId::PRIMARY).tokenize();
    let via_iter: Vec<_> = Lexer::new(src, FileId::PRIMARY).collect();
    assert_eq!(via_iter, via_tokenize);
}

#[test]
fn iterator_returns_none_after_eof() {
    let mut it = Lexer::new("int", FileId::PRIMARY);
    // int, Eof, then None forever.
    assert!(matches!(it.next().unwrap().kind, TokenKind::Int));
    assert!(matches!(it.next().unwrap().kind, TokenKind::Eof));
    assert!(it.next().is_none());
    assert!(it.next().is_none());
}

// =====================================================================
// Real-ish snippets
// =====================================================================

#[test]
fn small_function_like_snippet() {
    let src = "int main() { return; }";
    let toks = kinds(src);
    assert_eq!(
        toks,
        vec![
            TokenKind::Int,
            TokenKind::Identifier("main".to_string()),
            TokenKind::LeftParen,
            TokenKind::RightParen,
            TokenKind::LeftBrace,
            TokenKind::Return,
            TokenKind::Semicolon,
            TokenKind::RightBrace,
        ]
    );
}

#[test]
fn preprocessor_directive_shape() {
    // The lexer itself doesn't interpret directives, but the hash must
    // be flagged as at_start_of_line so the preprocessor can pick it up.
    let toks = lex("#include");
    assert_eq!(toks.len(), 2);
    assert!(matches!(toks[0].kind, TokenKind::Hash));
    assert!(toks[0].at_start_of_line);
    assert_eq!(toks[1].kind, TokenKind::Identifier("include".to_string()));
    assert!(!toks[1].at_start_of_line);
}

#[test]
fn expression_with_numbers_and_operators() {
    // `x = 3 + 4 * 0x10;`
    let (toks, diags) = lex_with_diags("x = 3 + 4 * 0x10;");
    assert!(diags.is_empty());
    let kinds: Vec<TokenKind> = toks.into_iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Identifier("x".to_string()),
            TokenKind::Equal,
            TokenKind::IntegerLiteral {
                value: 3,
                suffix: IntSuffix::None
            },
            TokenKind::Plus,
            TokenKind::IntegerLiteral {
                value: 4,
                suffix: IntSuffix::None
            },
            TokenKind::Star,
            TokenKind::IntegerLiteral {
                value: 16,
                suffix: IntSuffix::None
            },
            TokenKind::Semicolon,
        ]
    );
}

#[test]
fn return_zero_snippet() {
    // `return 0;` — makes sure the trivial case lexes right.
    let (toks, _) = lex_with_diags("return 0;");
    let kinds: Vec<TokenKind> = toks.into_iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Return,
            TokenKind::IntegerLiteral {
                value: 0,
                suffix: IntSuffix::None
            },
            TokenKind::Semicolon,
        ]
    );
}

// =====================================================================
// lex_fragment
// =====================================================================

#[test]
fn lex_fragment_strips_trailing_eof() {
    let toks = lex_fragment("foo");
    assert_eq!(toks.len(), 1);
    assert!(matches!(toks[0].kind, TokenKind::Identifier(ref s) if s == "foo"));
}

#[test]
fn lex_fragment_of_empty_string_is_empty() {
    assert!(lex_fragment("").is_empty());
}

#[test]
fn lex_fragment_handles_punctuation_and_literals() {
    let toks = lex_fragment("12 + 3");
    assert_eq!(toks.len(), 3);
    assert!(matches!(
        toks[0].kind,
        TokenKind::IntegerLiteral { value: 12, .. }
    ));
    assert!(matches!(toks[1].kind, TokenKind::Plus));
    assert!(matches!(
        toks[2].kind,
        TokenKind::IntegerLiteral { value: 3, .. }
    ));
}

#[test]
fn lex_fragment_of_concatenation_result_produces_single_identifier() {
    // Simulates what the preprocessor does for `a##b` → `ab`.
    let toks = lex_fragment("ab");
    assert_eq!(toks.len(), 1);
    assert!(matches!(toks[0].kind, TokenKind::Identifier(ref s) if s == "ab"));
}

#[test]
fn lex_fragment_of_two_tokens_reports_both() {
    // `12` `+` produces two tokens — useful for the preprocessor to
    // notice that a paste result is not a single valid preprocessing
    // token and warn.
    let toks = lex_fragment("1 2");
    assert_eq!(toks.len(), 2);
}

// =====================================================================
// Part 3 — edge cases and error recovery: NO panics on any input
// =====================================================================

#[test]
fn part3_unterminated_string_recovers() {
    let (toks, diags) = lex_with_diags("\"hello");
    assert_eq!(toks.len(), 1);
    assert!(matches!(toks[0].kind, TokenKind::StringLiteral { .. }));
    assert!(diags
        .iter()
        .any(|d| d.message.contains("unterminated string literal")),);
}

#[test]
fn part3_unterminated_char_recovers() {
    let (toks, diags) = lex_with_diags("'a");
    assert_eq!(toks.len(), 1);
    assert!(matches!(toks[0].kind, TokenKind::CharLiteral { .. }));
    assert!(diags
        .iter()
        .any(|d| d.message.contains("unterminated character constant")),);
}

#[test]
fn part3_unterminated_block_comment_reaches_eof_without_panic() {
    let mut lx = Lexer::new("/* this never ends", FileId::PRIMARY);
    let toks = lx.tokenize();
    assert_eq!(toks.len(), 1);
    assert!(matches!(toks[0].kind, TokenKind::Eof));
}

#[test]
fn part3_invalid_octal_emits_diagnostic() {
    let (_, diags) = lex_with_diags("0889");
    assert!(diags
        .iter()
        .any(|d| d.message.contains("invalid digit in octal")),);
}

#[test]
fn part3_unknown_escape_sequence_is_warning_not_panic() {
    let (toks, diags) = lex_with_diags(r#""\q""#);
    assert_eq!(toks.len(), 1);
    assert!(diags.iter().any(|d| d.message.contains("unknown escape")));
}

#[test]
fn part3_empty_char_literal_emits_diagnostic() {
    let (toks, diags) = lex_with_diags("''");
    assert_eq!(toks.len(), 1);
    assert!(diags
        .iter()
        .any(|d| d.message.contains("empty character constant")),);
}

#[test]
fn part3_null_byte_in_source_does_not_panic() {
    let src = "int\0x";
    let (toks, _diags) = lex_with_diags(src);
    // The null byte lexes as Unknown('\0') between `int` and `x`.
    assert!(toks
        .iter()
        .any(|t| matches!(t.kind, TokenKind::Unknown('\0'))));
    // Lexing completed without panicking.
}

#[test]
fn part3_very_long_identifier_does_not_crash() {
    let src: String = "a".repeat(1000);
    let (toks, diags) = lex_with_diags(&src);
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 1);
    match &toks[0].kind {
        TokenKind::Identifier(s) => assert_eq!(s.len(), 1000),
        other => panic!("expected Identifier, got {other:?}"),
    }
}

#[test]
fn part3_very_long_string_literal_does_not_crash() {
    let body: String = "a".repeat(10_000);
    let src = format!("\"{body}\"");
    let (toks, diags) = lex_with_diags(&src);
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 1);
    match &toks[0].kind {
        TokenKind::StringLiteral { value, .. } => assert_eq!(value.len(), 10_000),
        other => panic!("expected StringLiteral, got {other:?}"),
    }
}

#[test]
fn part3_backslash_at_end_of_file_is_never_a_panic() {
    // `\` with nothing after is not a valid line continuation (no newline)
    // but the lexer must return a token instead of crashing.
    let mut lx = Lexer::new("int x = 1;\\", FileId::PRIMARY);
    let toks = lx.tokenize();
    let _ = lx.take_diagnostics();
    assert!(matches!(toks.last().unwrap().kind, TokenKind::Eof));
}

#[test]
fn part3_garbage_bytes_do_not_panic() {
    // A deliberately nasty mix of ASCII punctuators, high bytes, nulls,
    // and control characters.  The only invariant tested here is that
    // `tokenize` returns and ends with Eof.
    let nasty: Vec<u8> = (0u8..=255u8).collect();
    if let Ok(s) = std::str::from_utf8(&nasty) {
        // For ASCII-clean 0..127 we can feed directly.
        let toks = Lexer::new(s, FileId::PRIMARY).tokenize();
        assert!(matches!(toks.last().unwrap().kind, TokenKind::Eof));
    }
    // Feed a string that includes valid UTF-8 but lots of structurally
    // odd characters.  The lexer should return.
    let odd = "\u{0000}\u{0001}\u{0007}\u{001B}\u{00A0}é漢字\u{10FFFF}";
    let toks = Lexer::new(odd, FileId::PRIMARY).tokenize();
    assert!(matches!(toks.last().unwrap().kind, TokenKind::Eof));
}

// =====================================================================
// Part 4 — Preprocessor readiness: lone backslash at EOF
// =====================================================================

#[test]
fn part4_backslash_alone_at_eof_does_not_splice() {
    // No newline follows the `\` — it is just an Unknown token.
    let (toks, _) = lex_with_diags("int x = 1;\\");
    // The terminal `\` is Unknown('\\').
    assert!(matches!(
        toks.last().unwrap().kind,
        TokenKind::Unknown('\\'),
    ));
}
