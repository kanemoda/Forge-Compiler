//! Character and string literal lexing: every C17 prefix, every escape
//! family, UTF-8 encoding of UCNs, line continuation inside literals,
//! and the error-recovery shape for unterminated or malformed literals.

use super::helpers::*;
use crate::{CharPrefix, Lexer, Span, StringPrefix, TokenKind};

// =====================================================================
// Character literals
// =====================================================================

// ---------- Basic shapes and prefixes ----------

#[test]
fn simple_char_literal() {
    let (v, p) = as_char(&single_clean("'A'"));
    assert_eq!(v, 0x41);
    assert_eq!(p, CharPrefix::None);
}

#[test]
fn wide_l_char_literal() {
    let (v, p) = as_char(&single_clean("L'A'"));
    assert_eq!(v, 0x41);
    assert_eq!(p, CharPrefix::L);
}

#[test]
fn utf16_lowercase_u_char_literal() {
    let (v, p) = as_char(&single_clean("u'A'"));
    assert_eq!(v, 0x41);
    assert_eq!(p, CharPrefix::U16);
}

#[test]
fn utf32_uppercase_u_char_literal() {
    let (v, p) = as_char(&single_clean("U'A'"));
    assert_eq!(v, 0x41);
    assert_eq!(p, CharPrefix::U32);
}

#[test]
fn char_literal_span_includes_prefix_and_quotes() {
    let (toks, diags) = lex_with_diags("L'A'");
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 1);
    assert_eq!(toks[0].span, Span::primary(0, 4));
}

// ---------- Simple escape sequences (every one) ----------

#[test]
fn all_simple_escape_sequences() {
    let cases: &[(&str, u32)] = &[
        (r"'\a'", 0x07),
        (r"'\b'", 0x08),
        (r"'\f'", 0x0C),
        (r"'\n'", 0x0A),
        (r"'\r'", 0x0D),
        (r"'\t'", 0x09),
        (r"'\v'", 0x0B),
        (r"'\\'", 0x5C),
        (r"'\''", 0x27),
        (r#"'\"'"#, 0x22),
        (r"'\?'", 0x3F),
    ];
    for (src, expected) in cases {
        let (v, p) = as_char(&single_clean(src));
        assert_eq!(v, *expected, "mis-lexed escape `{src}`");
        assert_eq!(p, CharPrefix::None);
    }
}

// ---------- Octal escapes ----------

#[test]
fn octal_null_escape() {
    let (v, _) = as_char(&single_clean(r"'\0'"));
    assert_eq!(v, 0);
}

#[test]
fn octal_escape_one_digit() {
    let (v, _) = as_char(&single_clean(r"'\7'"));
    assert_eq!(v, 0o7);
}

#[test]
fn octal_escape_two_digits() {
    let (v, _) = as_char(&single_clean(r"'\12'"));
    assert_eq!(v, 0o12);
}

#[test]
fn octal_escape_three_digits() {
    let (v, _) = as_char(&single_clean(r"'\377'"));
    assert_eq!(v, 0o377);
}

#[test]
fn octal_escape_stops_at_three_digits() {
    // `\1234` is `\123` (value 0o123 = 83) followed by `'4'` (0x34 = 52).
    // So `'\1234'` is a multi-character narrow constant: (0o123 << 8) | '4'.
    let (v, _) = as_char(&single_clean(r"'\1234'"));
    assert_eq!(v, (0o123u32 << 8) | b'4' as u32);
}

#[test]
fn octal_escape_stops_at_non_octal_digit() {
    // `\19` is octal `\1` (value 1) then literal `9`.
    let (v, _) = as_char(&single_clean(r"'\19'"));
    assert_eq!(v, (1u32 << 8) | b'9' as u32);
}

#[test]
fn octal_escape_boundary_zero_and_max() {
    let (v0, _) = as_char(&single_clean(r"'\0'"));
    assert_eq!(v0, 0);
    let (vmax, _) = as_char(&single_clean(r"'\377'"));
    assert_eq!(vmax, 255);
}

// ---------- Hex escapes ----------

#[test]
fn hex_escape_ascii_a() {
    let (v, _) = as_char(&single_clean(r"'\x41'"));
    assert_eq!(v, 0x41);
}

#[test]
fn hex_escape_lowercase_and_uppercase_mixed() {
    let (v, _) = as_char(&single_clean(r"'\xFf'"));
    assert_eq!(v, 0xFF);
}

#[test]
fn hex_escape_one_digit() {
    let (v, _) = as_char(&single_clean(r"'\x1'"));
    assert_eq!(v, 0x1);
}

#[test]
fn hex_escape_many_digits_in_wide_literal() {
    // Wide-literal prefix lets the value exceed 8 bits cleanly.
    let (v, p) = as_char(&single_clean(r"U'\x12345678'"));
    assert_eq!(v, 0x1234_5678);
    assert_eq!(p, CharPrefix::U32);
}

#[test]
fn hex_escape_stops_at_non_hex() {
    // `"\x41g"` is byte 0x41 ('A') then literal 'g'.
    let (s, _) = as_string(&single_clean(r#""\x41g""#));
    assert_eq!(s, "Ag");
}

#[test]
fn hex_escape_without_digits_is_error() {
    let (toks, diags) = lex_with_diags(r#""\x""#);
    assert_eq!(toks.len(), 1);
    assert_eq!(as_string(&toks[0].kind).0, "");
    assert!(diags.iter().any(|d| d.message.contains("\\x")));
}

#[test]
fn hex_escape_overflow_emits_warning() {
    // 9 hex digits: 0x123456789 overflows 32 bits.
    let (_, diags) = lex_with_diags(r"U'\x123456789'");
    assert!(
        diags.iter().any(|d| d.message.contains("overflow")),
        "expected overflow warning for 9-digit hex escape, got {diags:?}"
    );
}

// ---------- Universal character names ----------

#[test]
fn ucn_u_exact_four_digits() {
    let (v, _) = as_char(&single_clean(r"U'\u4e2d'"));
    assert_eq!(v, 0x4E2D);
}

#[test]
fn ucn_big_u_exact_eight_digits() {
    let (v, _) = as_char(&single_clean(r"U'\U0001F600'"));
    assert_eq!(v, 0x1_F600);
}

#[test]
fn ucn_u_with_too_few_digits_is_error() {
    let (_, diags) = lex_with_diags(r"'\u12'");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("universal character name")),
        "got diagnostics: {diags:?}"
    );
}

#[test]
fn ucn_big_u_truncated_by_quote_is_error() {
    // `\U000041'` has only 6 hex digits before the closing quote.
    let (_, diags) = lex_with_diags(r"'\U000041'");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("universal character name")),
        "got diagnostics: {diags:?}"
    );
}

#[test]
fn ucn_with_non_hex_digit_is_error() {
    let (_, diags) = lex_with_diags(r"'\u00Gz'");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("universal character name")),
        "got diagnostics: {diags:?}"
    );
}

#[test]
fn ucn_encoded_into_utf8_string() {
    let (s, _) = as_string(&single_clean(r#""\u4e2d""#));
    // 0x4E2D is the Chinese character 中 — three bytes in UTF-8.
    assert_eq!(s, "\u{4e2d}");
    assert_eq!(s.len(), 3);
}

// ---------- Empty / multi-character / boundary cases ----------

#[test]
fn empty_char_literal_is_error() {
    let (toks, diags) = lex_with_diags("''");
    assert_eq!(toks.len(), 1);
    let (v, p) = as_char(&toks[0].kind);
    assert_eq!(v, 0, "empty literal still produces a token, value = 0");
    assert_eq!(p, CharPrefix::None);
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("empty character constant")),
        "got diagnostics: {diags:?}"
    );
}

#[test]
fn multi_character_narrow_constant_packs_bytes() {
    // 'ab' is implementation-defined but valid — GCC packs 'a' into
    // the high byte.  value = ('a' << 8) | 'b' = 0x6162 = 24930.
    let (v, p) = as_char(&single_clean("'ab'"));
    assert_eq!(v, 0x6162);
    assert_eq!(p, CharPrefix::None);
}

#[test]
fn multi_character_with_escapes() {
    // '\n\t' packs \n (0x0A) into the high byte and \t (0x09) in low.
    let (v, _) = as_char(&single_clean(r"'\n\t'"));
    assert_eq!(v, (0x0A_u32 << 8) | 0x09);
}

#[test]
fn four_byte_multichar_constant() {
    let (v, _) = as_char(&single_clean("'abcd'"));
    assert_eq!(
        v,
        (b'a' as u32) << 24 | (b'b' as u32) << 16 | (b'c' as u32) << 8 | b'd' as u32
    );
}

#[test]
fn multi_character_wide_constant_warns_and_returns_first() {
    let (toks, diags) = lex_with_diags("L'ab'");
    assert_eq!(toks.len(), 1);
    let (v, p) = as_char(&toks[0].kind);
    assert_eq!(v, b'a' as u32);
    assert_eq!(p, CharPrefix::L);
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("multi-character wide")),
        "got diagnostics: {diags:?}"
    );
}

// ---------- Error recovery ----------

#[test]
fn unterminated_char_literal_at_eof() {
    let (toks, diags) = lex_with_diags("'abc");
    assert_eq!(toks.len(), 1);
    let (v, _) = as_char(&toks[0].kind);
    // Three narrow chars packed: 'a','b','c'.
    assert_eq!(v, (b'a' as u32) << 16 | (b'b' as u32) << 8 | b'c' as u32);
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("unterminated character constant")),
        "got diagnostics: {diags:?}"
    );
}

#[test]
fn unterminated_char_literal_stops_at_newline() {
    // Scanner must not gobble the newline: subsequent tokens must
    // appear on the next line with `at_start_of_line` set.
    let (toks, diags) = lex_with_diags("'abc\nx");
    assert_eq!(toks.len(), 2);
    assert!(matches!(toks[0].kind, TokenKind::CharLiteral { .. }));
    assert_eq!(toks[1].kind, TokenKind::Identifier("x".to_string()));
    assert!(toks[1].at_start_of_line);
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("unterminated character constant")),
        "got diagnostics: {diags:?}"
    );
}

#[test]
fn invalid_escape_emits_diagnostic_but_literal_survives() {
    let (toks, diags) = lex_with_diags(r"'\q'");
    assert_eq!(toks.len(), 1);
    let (v, _) = as_char(&toks[0].kind);
    assert_eq!(
        v, b'q' as u32,
        "invalid escape falls back to literal char value"
    );
    assert!(
        diags.iter().any(|d| d.message.contains("unknown escape")),
        "got diagnostics: {diags:?}"
    );
}

#[test]
fn backslash_newline_in_char_literal_is_accepted_as_continuation() {
    // '\<newline>a' = line continuation + 'a' = value 'a'.
    let (toks, diags) = lex_with_diags("'\\\na'");
    assert_eq!(toks.len(), 1);
    let (v, _) = as_char(&toks[0].kind);
    assert_eq!(v, b'a' as u32);
    assert!(diags.is_empty(), "no diagnostics expected, got {diags:?}");
}

#[test]
fn lone_backslash_before_eof_in_char_literal() {
    let (toks, diags) = lex_with_diags(r"'\");
    assert_eq!(toks.len(), 1);
    assert!(
        diags.iter().any(|d| d.message.contains("incomplete escape")
            || d.message.contains("unterminated character constant")),
        "got diagnostics: {diags:?}"
    );
}

// =====================================================================
// String literals
// =====================================================================

// ---------- Basic shapes and prefixes ----------

#[test]
fn simple_string_literal() {
    let (s, p) = as_string(&single_clean(r#""hello""#));
    assert_eq!(s, "hello");
    assert_eq!(p, StringPrefix::None);
}

#[test]
fn empty_string_is_legal() {
    let (s, p) = as_string(&single_clean(r#""""#));
    assert_eq!(s, "");
    assert_eq!(p, StringPrefix::None);
}

#[test]
fn wide_l_string_literal() {
    let (s, p) = as_string(&single_clean(r#"L"hello""#));
    assert_eq!(s, "hello");
    assert_eq!(p, StringPrefix::L);
}

#[test]
fn utf8_string_literal() {
    let (s, p) = as_string(&single_clean(r#"u8"hello""#));
    assert_eq!(s, "hello");
    assert_eq!(p, StringPrefix::Utf8);
}

#[test]
fn utf16_string_literal() {
    let (s, p) = as_string(&single_clean(r#"u"hello""#));
    assert_eq!(s, "hello");
    assert_eq!(p, StringPrefix::U16);
}

#[test]
fn utf32_string_literal() {
    let (s, p) = as_string(&single_clean(r#"U"hello""#));
    assert_eq!(s, "hello");
    assert_eq!(p, StringPrefix::U32);
}

#[test]
fn string_literal_span_includes_prefix_and_quotes() {
    let (toks, _) = lex_with_diags(r#"u8"hi""#);
    assert_eq!(toks.len(), 1);
    assert_eq!(toks[0].span, Span::primary(0, 6));
}

// ---------- Escape sequences in strings ----------

#[test]
fn all_simple_escapes_in_string() {
    let (s, _) = as_string(&single_clean(r#""\a\b\f\n\r\t\v\\\'\"\?""#));
    assert_eq!(s, "\u{07}\u{08}\u{0C}\n\r\t\u{0B}\\\'\"?");
}

#[test]
fn octal_and_hex_escapes_in_string() {
    let (s, _) = as_string(&single_clean(r#""\101\x42""#));
    assert_eq!(s, "AB");
}

#[test]
fn universal_character_names_in_string() {
    let (s, _) = as_string(&single_clean(r#""\u00e9\U0001F600""#));
    assert_eq!(s, "\u{e9}\u{1F600}");
}

// ---------- Line continuation ----------

#[test]
fn line_continuation_joins_parts_of_string() {
    // "hello\<NL>world" — the backslash + newline should vanish.
    let (s, _) = as_string(&single_clean("\"hello\\\nworld\""));
    assert_eq!(s, "helloworld");
}

#[test]
fn line_continuation_with_crlf() {
    let (s, _) = as_string(&single_clean("\"ab\\\r\ncd\""));
    assert_eq!(s, "abcd");
}

// ---------- Error recovery ----------

#[test]
fn unterminated_string_at_eof() {
    let (toks, diags) = lex_with_diags(r#""hello"#);
    assert_eq!(toks.len(), 1);
    let (s, _) = as_string(&toks[0].kind);
    assert_eq!(s, "hello");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("unterminated string literal")),
        "got diagnostics: {diags:?}"
    );
}

#[test]
fn unterminated_string_stops_at_newline() {
    let (toks, diags) = lex_with_diags("\"hello\nworld\"");
    // First token: unterminated StringLiteral with body "hello".
    // Then `world`, then another unterminated string.
    assert!(!toks.is_empty());
    let (s0, _) = as_string(&toks[0].kind);
    assert_eq!(s0, "hello");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("unterminated string literal")),
        "got diagnostics: {diags:?}"
    );
    // The identifier `world` must appear on the next line.
    let world = toks
        .iter()
        .find(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "world"));
    assert!(
        world.is_some(),
        "expected `world` identifier after unterminated string, got {toks:?}"
    );
    assert!(world.unwrap().at_start_of_line);
}

#[test]
fn invalid_escape_in_string_emits_warning_but_keeps_literal() {
    let (toks, diags) = lex_with_diags(r#""\q""#);
    assert_eq!(toks.len(), 1);
    let (s, _) = as_string(&toks[0].kind);
    assert_eq!(s, "q");
    assert!(
        diags.iter().any(|d| d.message.contains("unknown escape")),
        "got diagnostics: {diags:?}"
    );
}

#[test]
fn string_with_leading_newline_produces_empty_string_and_diagnostic() {
    // `"` alone on a line, then newline — unterminated.
    let (toks, diags) = lex_with_diags("\"\n");
    assert!(!toks.is_empty());
    let (s, _) = as_string(&toks[0].kind);
    assert!(s.is_empty());
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("unterminated string literal")),
        "got diagnostics: {diags:?}"
    );
}

// ---------- Cross-cutting: prefixes must not be mis-lexed as identifiers ----------

#[test]
fn l_alone_is_still_an_identifier() {
    // `L` without a following quote is just the identifier `L`.
    let (toks, diags) = lex_with_diags("L");
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 1);
    assert_eq!(toks[0].kind, TokenKind::Identifier("L".to_string()));
}

#[test]
fn u_followed_by_ident_is_identifier() {
    // `u8abc` is a single identifier, not `u8` + identifier.
    let (toks, diags) = lex_with_diags("u8abc");
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 1);
    assert_eq!(toks[0].kind, TokenKind::Identifier("u8abc".to_string()));
}

#[test]
fn u8_with_apostrophe_is_identifier_then_char_literal() {
    // u8 is NOT a valid char-literal prefix — only a string prefix.
    // So `u8'x'` must lex as identifier `u8` then char literal `'x'`.
    let (toks, diags) = lex_with_diags("u8'x'");
    assert!(diags.is_empty(), "unexpected diags: {diags:?}");
    assert_eq!(toks.len(), 2);
    assert_eq!(toks[0].kind, TokenKind::Identifier("u8".to_string()));
    let (v, p) = as_char(&toks[1].kind);
    assert_eq!(v, b'x' as u32);
    assert_eq!(p, CharPrefix::None);
}

#[test]
fn adjacent_string_literals_produce_two_tokens() {
    // The lexer does not concatenate adjacent literals; that is a
    // translation-phase-6 job handled by the preprocessor/parser.
    let (toks, diags) = lex_with_diags(r#""foo" "bar""#);
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 2);
    assert_eq!(as_string(&toks[0].kind).0, "foo");
    assert_eq!(as_string(&toks[1].kind).0, "bar");
}

// ---------- Boundary: all prefixes × a representative escape ----------

#[test]
fn every_char_prefix_accepts_hex_escape() {
    for src in &[r"'\x41'", r"L'\x41'", r"u'\x41'", r"U'\x41'"] {
        let (v, _) = as_char(&single_clean(src));
        assert_eq!(v, 0x41, "mis-lexed `{src}`");
    }
}

#[test]
fn every_string_prefix_accepts_all_escape_families() {
    for src in &[
        r#""\n\x41\101\u00e9""#,
        r#"L"\n\x41\101\u00e9""#,
        r#"u8"\n\x41\101\u00e9""#,
        r#"u"\n\x41\101\u00e9""#,
        r#"U"\n\x41\101\u00e9""#,
    ] {
        let (s, _) = as_string(&single_clean(src));
        assert_eq!(s, "\nAA\u{e9}", "mis-lexed `{src}`");
    }
}

// =====================================================================
// Bare quote at EOF — unterminated literal recovery (from lexer.rs)
// =====================================================================

#[test]
fn isolated_quote_is_unterminated_literal() {
    // Phase 1.3: a bare `'` or `"` opens a character/string literal
    // that is immediately unterminated.  The lexer emits a diagnostic
    // and still produces the corresponding empty literal token.
    let mut lx = Lexer::new("'", FileId::PRIMARY);
    let toks = lx.tokenize();
    let diags = lx.take_diagnostics();
    assert_eq!(toks.len(), 2, "expected CharLiteral + Eof, got {toks:?}");
    assert!(matches!(
        toks[0].kind,
        TokenKind::CharLiteral {
            prefix: CharPrefix::None,
            ..
        }
    ));
    assert!(
        !diags.is_empty(),
        "expected diagnostic for unterminated `'`"
    );

    let mut lx = Lexer::new("\"", FileId::PRIMARY);
    let toks = lx.tokenize();
    let diags = lx.take_diagnostics();
    assert_eq!(toks.len(), 2, "expected StringLiteral + Eof, got {toks:?}");
    assert!(matches!(
        toks[0].kind,
        TokenKind::StringLiteral {
            prefix: StringPrefix::None,
            ..
        }
    ));
    assert!(
        !diags.is_empty(),
        "expected diagnostic for unterminated `\"`"
    );
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
