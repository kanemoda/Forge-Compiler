//! Integer literal lexing: decimal, octal, hex, every suffix combination,
//! and the disambiguation corners around trailing dots/identifiers.

use super::helpers::*;
use crate::{IntSuffix, Span, TokenKind};

#[test]
fn digits_tokenize_as_a_single_integer_literal() {
    // Phase 1.2: `12` is one integer literal, not two Unknown tokens.
    let toks = kinds("12");
    assert_eq!(
        toks,
        vec![TokenKind::IntegerLiteral {
            value: 12,
            suffix: IntSuffix::None,
        }]
    );
}

// =====================================================================
// Decimal integers
// =====================================================================

#[test]
fn single_zero_is_decimal_zero() {
    // "0" is decimal, not octal — it only has one digit.
    let (v, s) = as_int(&single_clean("0"));
    assert_eq!(v, 0);
    assert_eq!(s, IntSuffix::None);
}

#[test]
fn small_decimal_integer() {
    let (v, s) = as_int(&single_clean("42"));
    assert_eq!(v, 42);
    assert_eq!(s, IntSuffix::None);
}

#[test]
fn large_decimal_integer() {
    let (v, _) = as_int(&single_clean("1234567890"));
    assert_eq!(v, 1_234_567_890);
}

#[test]
fn u64_max_decimal() {
    let (v, _) = as_int(&single_clean("18446744073709551615"));
    assert_eq!(v, u64::MAX);
}

#[test]
fn decimal_overflow_emits_warning() {
    let (_, diags) = lex_with_diags("18446744073709551616"); // u64::MAX + 1
    assert!(
        diags.iter().any(|d| d.message.contains("too large")),
        "expected overflow warning, got {diags:?}"
    );
}

#[test]
fn very_long_decimal_overflow() {
    let (_, diags) = lex_with_diags("99999999999999999999999999999999");
    assert!(diags.iter().any(|d| d.message.contains("too large")));
}

// =====================================================================
// Octal integers
// =====================================================================

#[test]
fn two_digit_octal() {
    // `010` is octal 8.
    let (v, _) = as_int(&single_clean("010"));
    assert_eq!(v, 8);
}

#[test]
fn three_digit_octal() {
    // `0777` is octal 511.
    let (v, _) = as_int(&single_clean("0777"));
    assert_eq!(v, 0o777);
}

#[test]
fn octal_with_leading_zeros() {
    let (v, _) = as_int(&single_clean("0007"));
    assert_eq!(v, 7);
}

#[test]
fn octal_invalid_digit_eight_emits_error() {
    let (_, diags) = lex_with_diags("08");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("invalid digit in octal")),
        "expected invalid-octal error, got {diags:?}"
    );
}

#[test]
fn octal_invalid_digit_nine_emits_error() {
    let (_, diags) = lex_with_diags("09");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("invalid digit in octal")),
        "expected invalid-octal error, got {diags:?}"
    );
}

#[test]
fn leading_zero_then_dot_is_float_not_octal() {
    // `08.5` is a decimal float 8.5 — no octal error.
    let (toks, diags) = lex_with_diags("08.5");
    assert!(diags.is_empty(), "unexpected diags: {diags:?}");
    assert_eq!(toks.len(), 1);
    let (v, _) = as_float(&toks[0].kind);
    assert!((v - 8.5).abs() < 1e-12);
}

// =====================================================================
// Hex integers
// =====================================================================

#[test]
fn hex_lowercase_x() {
    let (v, _) = as_int(&single_clean("0x1F"));
    assert_eq!(v, 31);
}

#[test]
fn hex_uppercase_x() {
    let (v, _) = as_int(&single_clean("0Xdead"));
    assert_eq!(v, 0xDEAD);
}

#[test]
fn hex_mixed_case_digits() {
    let (v, _) = as_int(&single_clean("0xCaFeBaBe"));
    assert_eq!(v, 0xCAFE_BABE);
}

#[test]
fn hex_max_u64() {
    let (v, _) = as_int(&single_clean("0xFFFFFFFFFFFFFFFF"));
    assert_eq!(v, u64::MAX);
}

#[test]
fn hex_overflow_emits_warning() {
    let (_, diags) = lex_with_diags("0x10000000000000000"); // 2^64
    assert!(
        diags.iter().any(|d| d.message.contains("too large")),
        "expected overflow warning, got {diags:?}"
    );
}

#[test]
fn empty_hex_emits_error() {
    let (_, diags) = lex_with_diags("0x");
    assert!(
        diags.iter().any(|d| d.message.contains("no digits")),
        "expected no-digits error, got {diags:?}"
    );
}

// =====================================================================
// Integer suffixes
// =====================================================================

#[test]
fn suffix_u_both_cases() {
    assert_eq!(as_int(&single_clean("1u")).1, IntSuffix::U);
    assert_eq!(as_int(&single_clean("1U")).1, IntSuffix::U);
}

#[test]
fn suffix_l_both_cases() {
    assert_eq!(as_int(&single_clean("1l")).1, IntSuffix::L);
    assert_eq!(as_int(&single_clean("1L")).1, IntSuffix::L);
}

#[test]
fn suffix_ul_every_order_and_case() {
    for src in ["1ul", "1uL", "1Ul", "1UL", "1lu", "1lU", "1Lu", "1LU"] {
        assert_eq!(as_int(&single_clean(src)).1, IntSuffix::UL, "`{src}`");
    }
}

#[test]
fn suffix_ll_matching_case_only() {
    assert_eq!(as_int(&single_clean("1ll")).1, IntSuffix::LL);
    assert_eq!(as_int(&single_clean("1LL")).1, IntSuffix::LL);
}

#[test]
fn suffix_ll_mixed_case_is_not_ll() {
    // `1lL` → `1l` (L suffix) then identifier `L`.
    let (toks, diags) = lex_with_diags("1lL");
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 2);
    let (v, s) = as_int(&toks[0].kind);
    assert_eq!(v, 1);
    assert_eq!(s, IntSuffix::L);
    assert_eq!(toks[1].kind, TokenKind::Identifier("L".to_string()));
}

#[test]
fn suffix_ull_every_order() {
    for src in [
        "1ull", "1uLL", "1Ull", "1ULL", "1llu", "1llU", "1LLu", "1LLU",
    ] {
        assert_eq!(as_int(&single_clean(src)).1, IntSuffix::ULL, "`{src}`");
    }
}

#[test]
fn hex_with_suffix() {
    let (v, s) = as_int(&single_clean("0xFFull"));
    assert_eq!(v, 0xFF);
    assert_eq!(s, IntSuffix::ULL);
}

#[test]
fn octal_with_suffix() {
    let (v, s) = as_int(&single_clean("0777L"));
    assert_eq!(v, 0o777);
    assert_eq!(s, IntSuffix::L);
}

// =====================================================================
// Integer / dot disambiguation
// =====================================================================

#[test]
fn int_dot_method_splits_correctly() {
    // `1.method` → IntegerLiteral(1), Dot, Identifier("method").
    let (toks, diags) = lex_with_diags("1.method");
    assert!(diags.is_empty(), "unexpected diags: {diags:?}");
    assert_eq!(toks.len(), 3);
    let (v, _) = as_int(&toks[0].kind);
    assert_eq!(v, 1);
    assert_eq!(toks[1].kind, TokenKind::Dot);
    assert_eq!(toks[2].kind, TokenKind::Identifier("method".to_string()));
}

#[test]
fn int_dot_underscore_splits_correctly() {
    // `1._x` — underscore starts an identifier.
    let (toks, _) = lex_with_diags("1._x");
    assert_eq!(toks.len(), 3);
    assert!(matches!(
        toks[0].kind,
        TokenKind::IntegerLiteral { value: 1, .. }
    ));
    assert_eq!(toks[1].kind, TokenKind::Dot);
    assert_eq!(toks[2].kind, TokenKind::Identifier("_x".to_string()));
}

#[test]
fn int_then_semicolon() {
    // Common case: sanity.
    let (toks, _) = lex_with_diags("42;");
    assert_eq!(toks.len(), 2);
    assert!(matches!(
        toks[0].kind,
        TokenKind::IntegerLiteral { value: 42, .. }
    ));
    assert_eq!(toks[1].kind, TokenKind::Semicolon);
}

// =====================================================================
// Spans
// =====================================================================

#[test]
fn integer_literal_span_covers_value_and_suffix() {
    let (toks, _) = lex_with_diags("0xFFull");
    assert_eq!(toks[0].span, Span::new(0, 7));
}

// =====================================================================
// Extra edge cases flagged while closing out phase 1.2
// =====================================================================

#[test]
fn zero_with_every_integer_suffix() {
    // The decimal-zero path must honour suffixes just like non-zero.
    let cases: &[(&str, IntSuffix)] = &[
        ("0", IntSuffix::None),
        ("0u", IntSuffix::U),
        ("0U", IntSuffix::U),
        ("0l", IntSuffix::L),
        ("0L", IntSuffix::L),
        ("0ul", IntSuffix::UL),
        ("0UL", IntSuffix::UL),
        ("0lu", IntSuffix::UL),
        ("0ll", IntSuffix::LL),
        ("0LL", IntSuffix::LL),
        ("0ull", IntSuffix::ULL),
        ("0llu", IntSuffix::ULL),
        ("0LLU", IntSuffix::ULL),
    ];
    for (src, expected) in cases {
        let (v, s) = as_int(&single_clean(src));
        assert_eq!(v, 0, "`{src}` value");
        assert_eq!(s, *expected, "`{src}` suffix");
    }
}

#[test]
fn hex_int_ending_with_f_is_hex_digit_not_float_suffix() {
    // `0x1f` is the hex integer 31 — `f` is a hex digit here, not the
    // `f` float suffix.  This disambiguation falls out of the greedy
    // hex-digit run plus the fact that `f` is only a float suffix on
    // an actual float literal.
    let (v, s) = as_int(&single_clean("0x1f"));
    assert_eq!(v, 0x1f);
    assert_eq!(s, IntSuffix::None);
}

#[test]
fn hex_int_ending_with_l_is_hex_digit_run_then_l_suffix() {
    // `0x1L` — `L` is NOT a hex digit, so the hex-digit run ends at
    // `1` and `L` becomes the integer suffix.
    let (v, s) = as_int(&single_clean("0x1L"));
    assert_eq!(v, 1);
    assert_eq!(s, IntSuffix::L);
}

#[test]
fn decimal_then_hex_in_same_source() {
    // Sanity: numeric literals don't bleed across a whitespace boundary.
    let (toks, diags) = lex_with_diags("42 0xFF");
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 2);
    let (a, _) = as_int(&toks[0].kind);
    let (b, _) = as_int(&toks[1].kind);
    assert_eq!(a, 42);
    assert_eq!(b, 0xFF);
}

#[test]
fn int_run_followed_by_invalid_suffix_lexes_as_int_plus_ident() {
    // `1abc` is an integer `1` followed by identifier `abc`, not an
    // error.  (gcc emits an error in strict mode but the *lexer*
    // accepts it; a later phase may enforce semantics.)
    let (toks, diags) = lex_with_diags("1abc");
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 2);
    let (v, s) = as_int(&toks[0].kind);
    assert_eq!(v, 1);
    assert_eq!(s, IntSuffix::None);
    assert_eq!(toks[1].kind, TokenKind::Identifier("abc".to_string()));
}

#[test]
fn long_octal_exact_value() {
    // `0755` is the classic chmod bitset: 0o755 == 493.
    let (v, _) = as_int(&single_clean("0755"));
    assert_eq!(v, 0o755);
}

#[test]
fn octal_u64_max_boundary() {
    // 0o1777777777777777777777 == 2^64 - 1.
    let (v, _) = as_int(&single_clean("01777777777777777777777"));
    assert_eq!(v, u64::MAX);
}

#[test]
fn octal_overflow_emits_warning() {
    // 2^64 in octal needs one more digit.
    let (_, diags) = lex_with_diags("02000000000000000000000");
    assert!(
        diags.iter().any(|d| d.message.contains("too large")),
        "expected overflow warning, got {diags:?}"
    );
}

#[test]
fn double_suffix_only_consumes_valid_tail() {
    // `1ulL` — `ul` matches the UL pattern, the trailing `L` is an
    // identifier.  This pins the longest-match table ordering so a
    // reordering bug can't regress silently.
    let (toks, diags) = lex_with_diags("1ulL");
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 2);
    let (v, s) = as_int(&toks[0].kind);
    assert_eq!(v, 1);
    assert_eq!(s, IntSuffix::UL);
    assert_eq!(toks[1].kind, TokenKind::Identifier("L".to_string()));
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
