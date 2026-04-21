//! Punctuator recognition, longest-match selection, and adjacent-token runs.

use super::helpers::*;
use crate::{IntSuffix, Lexer, TokenKind};

#[test]
fn all_single_and_multi_char_punctuators() {
    let cases: &[(&str, TokenKind)] = &[
        // Brackets
        ("(", TokenKind::LeftParen),
        (")", TokenKind::RightParen),
        ("{", TokenKind::LeftBrace),
        ("}", TokenKind::RightBrace),
        ("[", TokenKind::LeftBracket),
        ("]", TokenKind::RightBracket),
        // Member access
        (".", TokenKind::Dot),
        ("->", TokenKind::Arrow),
        // Increment/decrement
        ("++", TokenKind::PlusPlus),
        ("--", TokenKind::MinusMinus),
        // Unary / binary operators
        ("&", TokenKind::Ampersand),
        ("*", TokenKind::Star),
        ("+", TokenKind::Plus),
        ("-", TokenKind::Minus),
        ("~", TokenKind::Tilde),
        ("!", TokenKind::Bang),
        ("/", TokenKind::Slash),
        ("%", TokenKind::Percent),
        // Shifts and comparisons
        ("<<", TokenKind::LessLess),
        (">>", TokenKind::GreaterGreater),
        ("<", TokenKind::Less),
        (">", TokenKind::Greater),
        ("<=", TokenKind::LessEqual),
        (">=", TokenKind::GreaterEqual),
        ("==", TokenKind::EqualEqual),
        ("!=", TokenKind::BangEqual),
        // Bitwise
        ("^", TokenKind::Caret),
        ("|", TokenKind::Pipe),
        // Logical
        ("&&", TokenKind::AmpAmp),
        ("||", TokenKind::PipePipe),
        // Ternary / labels / statements
        ("?", TokenKind::Question),
        (":", TokenKind::Colon),
        (";", TokenKind::Semicolon),
        // Ellipsis
        ("...", TokenKind::Ellipsis),
        // Assignment operators
        ("=", TokenKind::Equal),
        ("*=", TokenKind::StarEqual),
        ("/=", TokenKind::SlashEqual),
        ("%=", TokenKind::PercentEqual),
        ("+=", TokenKind::PlusEqual),
        ("-=", TokenKind::MinusEqual),
        ("<<=", TokenKind::LessLessEqual),
        (">>=", TokenKind::GreaterGreaterEqual),
        ("&=", TokenKind::AmpEqual),
        ("^=", TokenKind::CaretEqual),
        ("|=", TokenKind::PipeEqual),
        // Misc
        (",", TokenKind::Comma),
        ("#", TokenKind::Hash),
        ("##", TokenKind::HashHash),
    ];
    for (src, expected) in cases {
        let got = kinds(src);
        assert_eq!(got, vec![expected.clone()], "punctuator `{src}` mis-lexed");
    }
}

#[test]
fn longest_match_is_greedy() {
    // `>>=` must be a single GreaterGreaterEqual, not `>>` + `=`.
    assert_eq!(kinds(">>="), vec![TokenKind::GreaterGreaterEqual]);
    assert_eq!(kinds("<<="), vec![TokenKind::LessLessEqual]);
    // `...` is the ellipsis, `..` is not a valid punctuator (two dots).
    assert_eq!(kinds("..."), vec![TokenKind::Ellipsis]);
    assert_eq!(kinds(".."), vec![TokenKind::Dot, TokenKind::Dot]);
}

#[test]
fn shift_right_with_no_trailing_equal() {
    // `>>x` → `>>`, identifier `x`.
    let toks = kinds(">>x");
    assert_eq!(
        toks,
        vec![
            TokenKind::GreaterGreater,
            TokenKind::Identifier("x".to_string())
        ]
    );
}

#[test]
fn compound_expression() {
    // `a += b << 3;` → identifier, +=, identifier, <<, integer 3, ;
    let toks = kinds("a += b << 3;");
    assert_eq!(
        toks,
        vec![
            TokenKind::Identifier("a".to_string()),
            TokenKind::PlusEqual,
            TokenKind::Identifier("b".to_string()),
            TokenKind::LessLess,
            TokenKind::IntegerLiteral {
                value: 3,
                suffix: IntSuffix::None,
            },
            TokenKind::Semicolon,
        ]
    );
}

#[test]
fn dot_alone_is_dot_punctuator() {
    // Just `.` (no digit after) is the Dot punctuator.
    let toks = Lexer::new(".", FileId::PRIMARY).tokenize();
    assert_eq!(toks.len(), 2);
    assert_eq!(toks[0].kind, TokenKind::Dot);
}

#[test]
fn ellipsis_is_still_ellipsis() {
    let toks = Lexer::new("...", FileId::PRIMARY).tokenize();
    assert_eq!(toks.len(), 2);
    assert_eq!(toks[0].kind, TokenKind::Ellipsis);
}

#[test]
fn dot_followed_by_identifier_is_dot_then_identifier() {
    // `.x` — dot followed by identifier start, NOT a float.
    let (toks, diags) = lex_with_diags(".x");
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 2);
    assert_eq!(toks[0].kind, TokenKind::Dot);
    assert_eq!(toks[1].kind, TokenKind::Identifier("x".to_string()));
}

// =====================================================================
// Part 2b — every punctuator (46 in C17)
// =====================================================================

#[test]
fn part2b_every_punctuator_round_trips() {
    let cases: &[(&str, TokenKind)] = &[
        ("[", TokenKind::LeftBracket),
        ("]", TokenKind::RightBracket),
        ("(", TokenKind::LeftParen),
        (")", TokenKind::RightParen),
        ("{", TokenKind::LeftBrace),
        ("}", TokenKind::RightBrace),
        (".", TokenKind::Dot),
        ("->", TokenKind::Arrow),
        ("++", TokenKind::PlusPlus),
        ("--", TokenKind::MinusMinus),
        ("&", TokenKind::Ampersand),
        ("*", TokenKind::Star),
        ("+", TokenKind::Plus),
        ("-", TokenKind::Minus),
        ("~", TokenKind::Tilde),
        ("!", TokenKind::Bang),
        ("/", TokenKind::Slash),
        ("%", TokenKind::Percent),
        ("<<", TokenKind::LessLess),
        (">>", TokenKind::GreaterGreater),
        ("<", TokenKind::Less),
        (">", TokenKind::Greater),
        ("<=", TokenKind::LessEqual),
        (">=", TokenKind::GreaterEqual),
        ("==", TokenKind::EqualEqual),
        ("!=", TokenKind::BangEqual),
        ("^", TokenKind::Caret),
        ("|", TokenKind::Pipe),
        ("&&", TokenKind::AmpAmp),
        ("||", TokenKind::PipePipe),
        ("?", TokenKind::Question),
        (":", TokenKind::Colon),
        (";", TokenKind::Semicolon),
        ("...", TokenKind::Ellipsis),
        ("=", TokenKind::Equal),
        ("*=", TokenKind::StarEqual),
        ("/=", TokenKind::SlashEqual),
        ("%=", TokenKind::PercentEqual),
        ("+=", TokenKind::PlusEqual),
        ("-=", TokenKind::MinusEqual),
        ("<<=", TokenKind::LessLessEqual),
        (">>=", TokenKind::GreaterGreaterEqual),
        ("&=", TokenKind::AmpEqual),
        ("^=", TokenKind::CaretEqual),
        ("|=", TokenKind::PipeEqual),
        (",", TokenKind::Comma),
        ("#", TokenKind::Hash),
        ("##", TokenKind::HashHash),
    ];
    for (src, expected) in cases {
        let k = kinds(src);
        assert_eq!(k, vec![expected.clone()], "punctuator `{src}`");
    }
}

#[test]
fn part2b_ambiguous_sequences_pick_longest_match() {
    // `>>=` must be a single GreaterGreaterEqual.
    assert_eq!(kinds(">>="), vec![TokenKind::GreaterGreaterEqual]);
    // `<<=` must be a single LessLessEqual.
    assert_eq!(kinds("<<="), vec![TokenKind::LessLessEqual]);
    // `>>=>>=` is two GreaterGreaterEqual tokens.
    assert_eq!(
        kinds(">>=>>="),
        vec![
            TokenKind::GreaterGreaterEqual,
            TokenKind::GreaterGreaterEqual,
        ],
    );
    // `...` is Ellipsis, `..` is two Dots (not a punctuator).
    assert_eq!(kinds("..."), vec![TokenKind::Ellipsis]);
    assert_eq!(kinds(".."), vec![TokenKind::Dot, TokenKind::Dot]);
    // `->>` is Arrow then Greater, not Arrow then Greater-Greater.
    assert_eq!(kinds("->>"), vec![TokenKind::Arrow, TokenKind::Greater]);
}

#[test]
fn part2b_adjacent_punctuator_run() {
    // `a]<<=>>=...` → a, ], <<=, >>=, ...
    let k = kinds("a]<<=>>=...");
    assert_eq!(
        k,
        vec![
            TokenKind::Identifier("a".to_string()),
            TokenKind::RightBracket,
            TokenKind::LessLessEqual,
            TokenKind::GreaterGreaterEqual,
            TokenKind::Ellipsis,
        ],
    );
}

#[test]
fn part4_hash_and_hashhash_are_distinct_tokens() {
    let k = kinds("# ## #");
    assert_eq!(
        k,
        vec![TokenKind::Hash, TokenKind::HashHash, TokenKind::Hash],
    );
    // `a##b` (no spaces) is three tokens.
    let k2 = kinds("a##b");
    assert_eq!(
        k2,
        vec![
            TokenKind::Identifier("a".to_string()),
            TokenKind::HashHash,
            TokenKind::Identifier("b".to_string()),
        ],
    );
}
