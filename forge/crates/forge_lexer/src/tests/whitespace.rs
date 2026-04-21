//! Whitespace handling, span tracking, line-start / leading-space flags,
//! and backslash-newline line continuation.

use super::helpers::*;
use crate::{IntSuffix, Lexer, Span, Token, TokenKind};

// =====================================================================
// Span value type
// =====================================================================

#[test]
fn span_display() {
    assert_eq!(Span::primary(0, 0).to_string(), "0:0..0");
    assert_eq!(Span::primary(5, 10).to_string(), "0:5..10");
    assert_eq!(Span::primary(123, 456).to_string(), "0:123..456");
}

#[test]
fn span_len_and_is_empty() {
    assert_eq!(Span::primary(0, 0).len(), 0);
    assert!(Span::primary(0, 0).is_empty());
    assert_eq!(Span::primary(3, 7).len(), 4);
    assert!(!Span::primary(3, 7).is_empty());
}

#[test]
fn span_range() {
    let s = Span::primary(5, 10);
    assert_eq!(s.range(), 5_usize..10_usize);
}

// =====================================================================
// Whitespace-only input
// =====================================================================

#[test]
fn whitespace_only_emits_only_eof() {
    let toks = Lexer::new("   \t\n\r\n  ", FileId::PRIMARY).tokenize();
    assert_eq!(toks.len(), 1);
    assert!(matches!(toks[0].kind, TokenKind::Eof));
}

// =====================================================================
// Span tracking
// =====================================================================

#[test]
fn spans_are_byte_offsets() {
    let toks = lex("int x");
    assert_eq!(toks[0].span, Span::primary(0, 3), "`int` span");
    assert_eq!(toks[1].span, Span::primary(4, 5), "`x` span");
}

#[test]
fn eof_span_points_at_end_of_input() {
    let src = "int";
    let toks = Lexer::new(src, FileId::PRIMARY).tokenize();
    let eof = toks.last().unwrap();
    assert!(matches!(eof.kind, TokenKind::Eof));
    assert_eq!(eof.span, Span::primary(3, 3));
}

// =====================================================================
// Flag tracking — at_start_of_line and has_leading_space
// =====================================================================

#[test]
fn first_token_is_at_start_of_line() {
    let toks = lex("int");
    assert!(toks[0].at_start_of_line);
    assert!(!toks[0].has_leading_space);
}

#[test]
fn leading_whitespace_sets_has_leading_space_not_start_of_line_after_first_token() {
    let toks = lex("int x");
    // `int`: first token on its line, no leading space.
    assert!(toks[0].at_start_of_line);
    assert!(!toks[0].has_leading_space);
    // `x`: same line, leading space = true, not at start of line.
    assert!(!toks[1].at_start_of_line);
    assert!(toks[1].has_leading_space);
}

#[test]
fn newline_resets_start_of_line_for_next_token() {
    let toks = lex("int\nx");
    assert!(toks[0].at_start_of_line);
    assert!(
        toks[1].at_start_of_line,
        "token after `\\n` is first on its line"
    );
    assert!(toks[1].has_leading_space, "newline counts as leading space");
}

#[test]
fn leading_space_before_first_token() {
    // `   foo` — foo is still at start of line AND has leading space.
    let toks = lex("   foo");
    assert!(toks[0].at_start_of_line);
    assert!(toks[0].has_leading_space);
}

#[test]
fn comments_mark_leading_space() {
    // `/* c */int` — int has leading space (the comment), same line.
    let toks = lex("/* c */int");
    assert_eq!(toks.len(), 1);
    assert!(toks[0].at_start_of_line);
    assert!(toks[0].has_leading_space);
}

#[test]
fn crlf_is_handled_as_a_single_newline() {
    let toks = lex("a\r\nb");
    assert_eq!(
        toks.iter().map(|t| t.kind.clone()).collect::<Vec<_>>(),
        vec![
            TokenKind::Identifier("a".to_string()),
            TokenKind::Identifier("b".to_string()),
        ]
    );
    assert!(toks[1].at_start_of_line);
}

// =====================================================================
// Part 2h — whitespace and line tracking
// =====================================================================

#[test]
fn part2h_horizontal_whitespace_between_tokens_sets_leading_space() {
    let (toks, _) = lex_with_diags("int    x   ;");
    assert!(toks[0].at_start_of_line);
    assert!(!toks[0].has_leading_space);
    assert!(toks[1].has_leading_space);
    assert!(!toks[1].at_start_of_line);
    assert!(toks[2].has_leading_space);
}

#[test]
fn part2h_tab_between_tokens() {
    let (toks, _) = lex_with_diags("int\ty");
    assert_eq!(toks.len(), 2);
    assert!(toks[1].has_leading_space);
    assert!(!toks[1].at_start_of_line);
}

#[test]
fn part2h_blank_lines_then_token_sets_start_of_line() {
    let (toks, _) = lex_with_diags("\n\nint z;");
    assert!(toks[0].at_start_of_line);
    assert!(toks[0].has_leading_space); // newlines count as whitespace
}

// =====================================================================
// Part 3 — empty input is just Eof
// =====================================================================

#[test]
fn part3_empty_input_is_only_eof() {
    let toks = Lexer::new("", FileId::PRIMARY).tokenize();
    assert_eq!(toks.len(), 1);
    assert!(matches!(toks[0].kind, TokenKind::Eof));
}

// =====================================================================
// Part 4 — Preprocessor readiness (line-start / leading-space flags
// the preprocessor depends on)
// =====================================================================

#[test]
fn part4_hash_at_start_of_line_flag() {
    // The spec: first `#` has at_start_of_line, second `#` after leading
    // whitespace also starts a directive (leading whitespace is allowed),
    // the mid-line `#` does not.
    let (toks, _) = lex_with_diags("#define FOO 1\n  #define BAR 2\nint x = #;");
    let hashes: Vec<(usize, &Token)> = toks
        .iter()
        .enumerate()
        .filter(|(_, t)| matches!(t.kind, TokenKind::Hash))
        .collect();
    assert_eq!(hashes.len(), 3, "expected three `#` tokens");
    assert!(
        hashes[0].1.at_start_of_line,
        "first `#` is at start of line"
    );
    assert!(
        hashes[1].1.at_start_of_line,
        "indented `#` still starts a directive",
    );
    assert!(
        !hashes[2].1.at_start_of_line,
        "mid-line `#` is NOT a directive",
    );
}

#[test]
fn part4_has_leading_space_distinguishes_foo_bar_from_foobar() {
    // `FOO BAR` → two tokens, BAR has leading space.
    let (toks, _) = lex_with_diags("FOO BAR");
    assert_eq!(toks.len(), 2);
    assert!(!toks[0].has_leading_space);
    assert!(toks[1].has_leading_space);

    // `FOO  BAR` (two spaces) — BAR still has leading space.  The flag
    // is a boolean; two spaces is not distinguishable from one.
    let (toks2, _) = lex_with_diags("FOO  BAR");
    assert!(toks2[1].has_leading_space);

    // `FOOBAR` → single identifier.
    let (toks3, _) = lex_with_diags("FOOBAR");
    assert_eq!(toks3.len(), 1);
    assert_eq!(toks3[0].kind, TokenKind::Identifier("FOOBAR".to_string()),);
}

#[test]
fn part4_backslash_newline_line_continuation_between_tokens() {
    // The critical preprocessor requirement: line continuation merges
    // the two physical lines, so the `2` must lex as an integer on the
    // same logical line as the `+`.
    let (toks, diags) = lex_with_diags("int x = 1 + \\\n        2;");
    assert!(diags.is_empty(), "unexpected diags: {diags:?}");
    let ks: Vec<_> = toks.iter().map(|t| t.kind.clone()).collect();
    assert_eq!(
        ks,
        vec![
            TokenKind::Int,
            TokenKind::Identifier("x".to_string()),
            TokenKind::Equal,
            TokenKind::IntegerLiteral {
                value: 1,
                suffix: IntSuffix::None,
            },
            TokenKind::Plus,
            TokenKind::IntegerLiteral {
                value: 2,
                suffix: IntSuffix::None,
            },
            TokenKind::Semicolon,
        ],
    );
    // The `2` is on the same logical line: at_start_of_line must be false.
    let two = toks
        .iter()
        .find(|t| matches!(t.kind, TokenKind::IntegerLiteral { value: 2, .. }))
        .unwrap();
    assert!(!two.at_start_of_line, "spliced line is not a new line");
    assert!(
        two.has_leading_space,
        "whitespace around the splice survives"
    );
}

#[test]
fn part4_backslash_newline_with_crlf() {
    let (toks, diags) = lex_with_diags("int x = 1 + \\\r\n        2;");
    assert!(diags.is_empty(), "unexpected diags: {diags:?}");
    let ks: Vec<_> = toks.iter().map(|t| t.kind.clone()).collect();
    assert_eq!(
        ks,
        vec![
            TokenKind::Int,
            TokenKind::Identifier("x".to_string()),
            TokenKind::Equal,
            TokenKind::IntegerLiteral {
                value: 1,
                suffix: IntSuffix::None,
            },
            TokenKind::Plus,
            TokenKind::IntegerLiteral {
                value: 2,
                suffix: IntSuffix::None,
            },
            TokenKind::Semicolon,
        ],
    );
}

#[test]
fn part4_multi_line_define_via_continuations() {
    // A typical multi-line `#define` in real C: every continuation must
    // splice, and the Hash / Identifier / ... tokens must all sit on the
    // same logical line (so the preprocessor sees them as a single
    // directive).
    let src = "#define X(a, b) \\\n    do { \\\n        a + b; \\\n    } while (0)";
    let (toks, diags) = lex_with_diags(src);
    assert!(diags.is_empty(), "unexpected diags: {diags:?}");

    // Only the `#` itself is at start of line — every other token is
    // part of the spliced logical line.
    let hash_ct = toks.iter().filter(|t| t.at_start_of_line).count();
    assert_eq!(
        hash_ct, 1,
        "exactly one token (the leading `#`) starts a logical line",
    );
    assert!(matches!(toks[0].kind, TokenKind::Hash));
}
