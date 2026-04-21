//! Line comments, block comments, nesting (or lack thereof), and the
//! interactions between comments and surrounding tokens.

use super::helpers::*;
use crate::{Lexer, TokenKind};

#[test]
fn line_comment_is_skipped() {
    let toks = lex("// this is a comment");
    assert!(toks.is_empty());
}

#[test]
fn line_comment_terminated_by_newline_still_yields_following_tokens() {
    let toks = kinds("// comment\nint");
    assert_eq!(toks, vec![TokenKind::Int]);
}

#[test]
fn block_comment_is_skipped() {
    let toks = lex("/* hello world */");
    assert!(toks.is_empty());
}

#[test]
fn block_comment_surrounding_tokens() {
    let toks = kinds("int /* hi */ x");
    assert_eq!(
        toks,
        vec![TokenKind::Int, TokenKind::Identifier("x".to_string())]
    );
}

#[test]
fn block_comment_spanning_lines_sets_start_of_line() {
    let toks = lex("a /*\n*/ b");
    assert_eq!(toks.len(), 2);
    assert!(
        toks[1].at_start_of_line,
        "token after a multi-line block comment should be at start of line"
    );
    assert!(toks[1].has_leading_space);
}

#[test]
fn unterminated_block_comment_reaches_eof() {
    // No panic; current phase silently stops at EOF.
    let toks = Lexer::new("/* open ", FileId::PRIMARY).tokenize();
    assert_eq!(toks.len(), 1);
    assert!(matches!(toks[0].kind, TokenKind::Eof));
}

#[test]
fn c_does_not_nest_block_comments() {
    // "/* a /* b */" closes at the first `*/`, leaving " c */"
    // in the stream: identifier `c`, `*`, `/`.
    let toks = kinds("/* a /* b */ c */");
    assert_eq!(
        toks,
        vec![
            TokenKind::Identifier("c".to_string()),
            TokenKind::Star,
            TokenKind::Slash,
        ]
    );
}

// =====================================================================
// Part 2g — comments
// =====================================================================

#[test]
fn part2g_line_comment_does_not_emit_a_token() {
    let (toks, _) = lex_with_diags("// just a comment");
    assert!(toks.is_empty());
}

#[test]
fn part2g_block_comment_does_not_emit_a_token() {
    let (toks, _) = lex_with_diags("/* just a block */");
    assert!(toks.is_empty());
}

#[test]
fn part2g_block_comments_do_not_nest_in_c() {
    // `/* outer /* inner */` closes at the first `*/`, so the tail is
    // `x */` which tokenises as `x`, `*`, `/`.
    let k = kinds("/* outer /* inner */ x */");
    assert_eq!(
        k,
        vec![
            TokenKind::Identifier("x".to_string()),
            TokenKind::Star,
            TokenKind::Slash,
        ],
    );
}

#[test]
fn part2g_inline_block_comment_splits_tokens() {
    // `int/* c */y` must produce two separate tokens `Int` and `y` —
    // the comment separates them even with no surrounding whitespace.
    let k = kinds("int/* c */y");
    assert_eq!(
        k,
        vec![TokenKind::Int, TokenKind::Identifier("y".to_string())],
    );
}

#[test]
fn part2g_trailing_line_comment_preserves_next_line() {
    let (toks, _) = lex_with_diags("int x; // trailing\nint y;");
    let names: Vec<_> = toks.iter().map(|t| t.kind.clone()).collect();
    assert_eq!(
        names,
        vec![
            TokenKind::Int,
            TokenKind::Identifier("x".to_string()),
            TokenKind::Semicolon,
            TokenKind::Int,
            TokenKind::Identifier("y".to_string()),
            TokenKind::Semicolon,
        ],
    );
    // Second `int` is on its own line.
    assert!(toks[3].at_start_of_line);
}
