//! Pre-Phase-2 validation battery.
//!
//! These tests are a checkpoint in their own right: they exhaustively
//! cover every token family the preprocessor will consume and lock in
//! the behaviours listed in the Phase-1 validation prompt.  Each
//! sub-section maps to a numbered requirement so regressions surface
//! against the exact spec line that called them out.

#![cfg(test)]

use crate::{CharPrefix, FloatSuffix, IntSuffix, Lexer, StringPrefix, Token, TokenKind};

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn lex_with_diags(src: &str) -> (Vec<Token>, Vec<crate::Diagnostic>) {
    let mut lx = Lexer::new(src);
    let mut toks = lx.tokenize();
    let last = toks.pop().expect("Eof");
    assert!(matches!(last.kind, TokenKind::Eof));
    (toks, lx.take_diagnostics())
}

fn kinds_only(src: &str) -> Vec<TokenKind> {
    lex_with_diags(src).0.into_iter().map(|t| t.kind).collect()
}

// =====================================================================
// Part 2a — all 44 C17 keywords
// =====================================================================

#[test]
fn part2a_every_keyword_round_trips() {
    let cases: &[(&str, TokenKind)] = &[
        ("auto", TokenKind::Auto),
        ("break", TokenKind::Break),
        ("case", TokenKind::Case),
        ("char", TokenKind::Char),
        ("const", TokenKind::Const),
        ("continue", TokenKind::Continue),
        ("default", TokenKind::Default),
        ("do", TokenKind::Do),
        ("double", TokenKind::Double),
        ("else", TokenKind::Else),
        ("enum", TokenKind::Enum),
        ("extern", TokenKind::Extern),
        ("float", TokenKind::Float),
        ("for", TokenKind::For),
        ("goto", TokenKind::Goto),
        ("if", TokenKind::If),
        ("inline", TokenKind::Inline),
        ("int", TokenKind::Int),
        ("long", TokenKind::Long),
        ("register", TokenKind::Register),
        ("restrict", TokenKind::Restrict),
        ("return", TokenKind::Return),
        ("short", TokenKind::Short),
        ("signed", TokenKind::Signed),
        ("sizeof", TokenKind::Sizeof),
        ("static", TokenKind::Static),
        ("struct", TokenKind::Struct),
        ("switch", TokenKind::Switch),
        ("typedef", TokenKind::Typedef),
        ("union", TokenKind::Union),
        ("unsigned", TokenKind::Unsigned),
        ("void", TokenKind::Void),
        ("volatile", TokenKind::Volatile),
        ("while", TokenKind::While),
        ("_Alignas", TokenKind::Alignas),
        ("_Alignof", TokenKind::Alignof),
        ("_Atomic", TokenKind::Atomic),
        ("_Bool", TokenKind::Bool),
        ("_Complex", TokenKind::Complex),
        ("_Generic", TokenKind::Generic),
        ("_Imaginary", TokenKind::Imaginary),
        ("_Noreturn", TokenKind::Noreturn),
        ("_Static_assert", TokenKind::StaticAssert),
        ("_Thread_local", TokenKind::ThreadLocal),
    ];
    assert_eq!(cases.len(), 44, "C17 has exactly 44 keywords");
    for (src, expected) in cases {
        let k = kinds_only(src);
        assert_eq!(k, vec![expected.clone()], "keyword `{src}`");
    }
}

#[test]
fn part2a_lookup_keyword_is_exposed_for_preprocessor() {
    assert_eq!(crate::lookup_keyword("int"), Some(TokenKind::Int));
    assert_eq!(
        crate::lookup_keyword("_Static_assert"),
        Some(TokenKind::StaticAssert),
    );
    assert_eq!(crate::lookup_keyword("Int"), None);
    assert_eq!(crate::lookup_keyword("FOO"), None);
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
        let k = kinds_only(src);
        assert_eq!(k, vec![expected.clone()], "punctuator `{src}`");
    }
}

#[test]
fn part2b_ambiguous_sequences_pick_longest_match() {
    // `>>=` must be a single GreaterGreaterEqual.
    assert_eq!(kinds_only(">>="), vec![TokenKind::GreaterGreaterEqual]);
    // `<<=` must be a single LessLessEqual.
    assert_eq!(kinds_only("<<="), vec![TokenKind::LessLessEqual]);
    // `>>=>>=` is two GreaterGreaterEqual tokens.
    assert_eq!(
        kinds_only(">>=>>="),
        vec![
            TokenKind::GreaterGreaterEqual,
            TokenKind::GreaterGreaterEqual,
        ],
    );
    // `...` is Ellipsis, `..` is two Dots (not a punctuator).
    assert_eq!(kinds_only("..."), vec![TokenKind::Ellipsis]);
    assert_eq!(kinds_only(".."), vec![TokenKind::Dot, TokenKind::Dot]);
    // `->>` is Arrow then Greater, not Arrow then Greater-Greater.
    assert_eq!(
        kinds_only("->>"),
        vec![TokenKind::Arrow, TokenKind::Greater]
    );
}

#[test]
fn part2b_adjacent_punctuator_run() {
    // `a]<<=>>=...` → a, ], <<=, >>=, ...
    let k = kinds_only("a]<<=>>=...");
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

// =====================================================================
// Part 2c — integer literals: every form from the spec
// =====================================================================

fn expect_int(src: &str, value: u64, suffix: IntSuffix) {
    let (toks, diags) = lex_with_diags(src);
    assert!(diags.is_empty(), "unexpected diags for `{src}`: {diags:?}");
    assert_eq!(toks.len(), 1, "expected 1 token for `{src}`, got {toks:?}");
    match &toks[0].kind {
        TokenKind::IntegerLiteral {
            value: v,
            suffix: s,
        } => {
            assert_eq!(*v, value, "`{src}` value");
            assert_eq!(*s, suffix, "`{src}` suffix");
        }
        other => panic!("`{src}` → {other:?}"),
    }
}

#[test]
fn part2c_every_integer_form() {
    expect_int("0", 0, IntSuffix::None);
    expect_int("42", 42, IntSuffix::None);
    expect_int("0777", 0o777, IntSuffix::None);
    expect_int("0xFF", 0xFF, IntSuffix::None);
    expect_int("0XAB", 0xAB, IntSuffix::None);
    expect_int("42u", 42, IntSuffix::U);
    expect_int("42U", 42, IntSuffix::U);
    expect_int("42l", 42, IntSuffix::L);
    expect_int("42L", 42, IntSuffix::L);
    expect_int("42ul", 42, IntSuffix::UL);
    expect_int("42UL", 42, IntSuffix::UL);
    expect_int("42lu", 42, IntSuffix::UL);
    expect_int("42LU", 42, IntSuffix::UL);
    expect_int("42ll", 42, IntSuffix::LL);
    expect_int("42LL", 42, IntSuffix::LL);
    expect_int("42ull", 42, IntSuffix::ULL);
    expect_int("42ULL", 42, IntSuffix::ULL);
    expect_int("42llu", 42, IntSuffix::ULL);
    expect_int("42LLU", 42, IntSuffix::ULL);
    expect_int("0xFFFFFFFFFFFFFFFF", u64::MAX, IntSuffix::None);
    expect_int("18446744073709551615", u64::MAX, IntSuffix::None);
}

#[test]
fn part2c_u64_plus_one_warns_overflow() {
    // One more than u64::MAX: the spec explicitly asks whether this warns.
    let (_toks, diags) = lex_with_diags("18446744073709551616");
    assert!(
        diags.iter().any(|d| d.message.contains("too large")),
        "expected overflow warning, got {diags:?}",
    );
}

// =====================================================================
// Part 2d — float literals: every form from the spec
// =====================================================================

fn expect_float_approx(src: &str, expected: f64, suffix: FloatSuffix) {
    let (toks, diags) = lex_with_diags(src);
    assert!(diags.is_empty(), "unexpected diags for `{src}`: {diags:?}");
    assert_eq!(toks.len(), 1, "expected 1 token for `{src}`, got {toks:?}");
    match &toks[0].kind {
        TokenKind::FloatLiteral { value, suffix: s } => {
            assert!(
                (value - expected).abs() < (expected.abs() * 1e-12).max(1e-12),
                "`{src}` value: got {value}, expected {expected}",
            );
            assert_eq!(*s, suffix, "`{src}` suffix");
        }
        other => panic!("`{src}` → {other:?}"),
    }
}

#[test]
fn part2d_every_float_form() {
    expect_float_approx("1.0", 1.0, FloatSuffix::None);
    expect_float_approx(".5", 0.5, FloatSuffix::None);
    expect_float_approx("1.", 1.0, FloatSuffix::None);
    expect_float_approx("1e10", 1e10, FloatSuffix::None);
    expect_float_approx("1E10", 1e10, FloatSuffix::None);
    expect_float_approx("1.5e-3", 1.5e-3, FloatSuffix::None);
    expect_float_approx("1.5e+3", 1.5e+3, FloatSuffix::None);
    expect_float_approx(".5e2", 50.0, FloatSuffix::None);
    expect_float_approx("0x1.0p10", 1024.0, FloatSuffix::None);
    expect_float_approx("0x1p10", 1024.0, FloatSuffix::None);
    // 0xA.Bp-3 = (10 + 11/16) * 2^-3 = 10.6875 / 8 = 1.3359375
    expect_float_approx("0xA.Bp-3", 1.3359375, FloatSuffix::None);
    expect_float_approx("1.0f", 1.0, FloatSuffix::F);
    expect_float_approx("1.0F", 1.0, FloatSuffix::F);
    expect_float_approx("1.0l", 1.0, FloatSuffix::L);
    expect_float_approx("1.0L", 1.0, FloatSuffix::L);
    expect_float_approx("1e10f", 1e10, FloatSuffix::F);
    expect_float_approx("0x1p10f", 1024.0, FloatSuffix::F);
}

// =====================================================================
// Part 2e — character literals: every form from the spec
// =====================================================================

fn expect_char(src: &str, value: u32, prefix: CharPrefix) {
    let (toks, diags) = lex_with_diags(src);
    assert!(diags.is_empty(), "unexpected diags for `{src}`: {diags:?}");
    assert_eq!(toks.len(), 1, "expected 1 token for `{src}`, got {toks:?}");
    match &toks[0].kind {
        TokenKind::CharLiteral {
            value: v,
            prefix: p,
        } => {
            assert_eq!(*v, value, "`{src}` value");
            assert_eq!(*p, prefix, "`{src}` prefix");
        }
        other => panic!("`{src}` → {other:?}"),
    }
}

#[test]
fn part2e_every_character_form() {
    expect_char("'a'", b'a' as u32, CharPrefix::None);
    expect_char(r"'\n'", 0x0A, CharPrefix::None);
    expect_char(r"'\t'", 0x09, CharPrefix::None);
    expect_char(r"'\\'", 0x5C, CharPrefix::None);
    expect_char(r"'\''", 0x27, CharPrefix::None);
    expect_char("'\\\"'", 0x22, CharPrefix::None);
    expect_char(r"'\0'", 0, CharPrefix::None);
    expect_char(r"'\a'", 0x07, CharPrefix::None);
    expect_char(r"'\b'", 0x08, CharPrefix::None);
    expect_char(r"'\f'", 0x0C, CharPrefix::None);
    expect_char(r"'\r'", 0x0D, CharPrefix::None);
    expect_char(r"'\v'", 0x0B, CharPrefix::None);
    expect_char(r"'\?'", 0x3F, CharPrefix::None);
    expect_char(r"'\x41'", 0x41, CharPrefix::None);
    expect_char(r"'\012'", 0o12, CharPrefix::None);
    expect_char(r"'\xFF'", 0xFF, CharPrefix::None);
    expect_char("L'a'", b'a' as u32, CharPrefix::L);
    expect_char("u'a'", b'a' as u32, CharPrefix::U16);
    expect_char("U'a'", b'a' as u32, CharPrefix::U32);
    expect_char(r"'\u0041'", 0x41, CharPrefix::None);
    expect_char(r"'\U00000041'", 0x41, CharPrefix::None);
    // multi-character narrow literal — implementation-defined but valid.
    expect_char(
        "'ab'",
        ((b'a' as u32) << 8) | (b'b' as u32),
        CharPrefix::None,
    );
}

// =====================================================================
// Part 2f — string literals: every form from the spec
// =====================================================================

fn expect_string(src: &str, value: &str, prefix: StringPrefix) {
    let (toks, diags) = lex_with_diags(src);
    assert!(diags.is_empty(), "unexpected diags for `{src}`: {diags:?}");
    assert_eq!(toks.len(), 1, "expected 1 token for `{src}`, got {toks:?}");
    match &toks[0].kind {
        TokenKind::StringLiteral {
            value: v,
            prefix: p,
        } => {
            assert_eq!(v, value, "`{src}` value");
            assert_eq!(*p, prefix, "`{src}` prefix");
        }
        other => panic!("`{src}` → {other:?}"),
    }
}

#[test]
fn part2f_every_string_form() {
    expect_string(r#""hello""#, "hello", StringPrefix::None);
    expect_string(r#""""#, "", StringPrefix::None);
    expect_string(r#""hello\nworld""#, "hello\nworld", StringPrefix::None);
    expect_string(r#""hello\tworld""#, "hello\tworld", StringPrefix::None);
    expect_string(r#""\\""#, "\\", StringPrefix::None);
    expect_string(r#""\"""#, "\"", StringPrefix::None);
    expect_string(r#""\x41\x42\x43""#, "ABC", StringPrefix::None);
    expect_string(r#""\012""#, "\n", StringPrefix::None); // \012 == '\n'
    expect_string(r#"L"wide""#, "wide", StringPrefix::L);
    expect_string(r#"u8"utf8""#, "utf8", StringPrefix::Utf8);
    expect_string(r#"u"utf16""#, "utf16", StringPrefix::U16);
    expect_string(r#"U"utf32""#, "utf32", StringPrefix::U32);
    // Line continuation inside string literal: "line1\<NL>line2" → "line1line2".
    expect_string("\"line1\\\nline2\"", "line1line2", StringPrefix::None);
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
    let k = kinds_only("/* outer /* inner */ x */");
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
    let k = kinds_only("int/* c */y");
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
    let mut lx = Lexer::new("/* this never ends");
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
    let mut lx = Lexer::new("int x = 1;\\");
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
        let toks = Lexer::new(s).tokenize();
        assert!(matches!(toks.last().unwrap().kind, TokenKind::Eof));
    }
    // Feed a string that includes valid UTF-8 but lots of structurally
    // odd characters.  The lexer should return.
    let odd = "\u{0000}\u{0001}\u{0007}\u{001B}\u{00A0}é漢字\u{10FFFF}";
    let toks = Lexer::new(odd).tokenize();
    assert!(matches!(toks.last().unwrap().kind, TokenKind::Eof));
}

#[test]
fn part3_empty_input_is_only_eof() {
    let toks = Lexer::new("").tokenize();
    assert_eq!(toks.len(), 1);
    assert!(matches!(toks[0].kind, TokenKind::Eof));
}

// =====================================================================
// Part 4 — Preprocessor readiness
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
fn part4_hash_and_hashhash_are_distinct_tokens() {
    let k = kinds_only("# ## #");
    assert_eq!(
        k,
        vec![TokenKind::Hash, TokenKind::HashHash, TokenKind::Hash],
    );
    // `a##b` (no spaces) is three tokens.
    let k2 = kinds_only("a##b");
    assert_eq!(
        k2,
        vec![
            TokenKind::Identifier("a".to_string()),
            TokenKind::HashHash,
            TokenKind::Identifier("b".to_string()),
        ],
    );
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
