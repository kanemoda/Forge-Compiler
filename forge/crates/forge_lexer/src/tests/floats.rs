//! Float literal lexing: decimal, hex, every suffix, and the integer/float
//! disambiguation corners that produce floats.

use super::helpers::*;
use crate::{FloatSuffix, Span, TokenKind};

// =====================================================================
// Decimal floats
// =====================================================================

#[test]
fn simple_decimal_float() {
    let (v, s) = as_float(&single_clean("1.5"));
    assert!((v - 1.5).abs() < 1e-12);
    assert_eq!(s, FloatSuffix::None);
}

#[test]
fn trailing_dot_float() {
    // "1." is a valid float (value 1.0) — the task explicitly lists
    // it as a legal form.
    let (v, _) = as_float(&single_clean("1."));
    assert!((v - 1.0).abs() < 1e-12);
}

#[test]
fn leading_dot_float() {
    let (v, _) = as_float(&single_clean(".5"));
    assert!((v - 0.5).abs() < 1e-12);
}

#[test]
fn float_with_positive_exponent() {
    let (v, _) = as_float(&single_clean("1e10"));
    assert!((v - 1e10).abs() < 1.0);
}

#[test]
fn float_with_negative_exponent() {
    let (v, _) = as_float(&single_clean("1.5e-3"));
    assert!((v - 1.5e-3).abs() < 1e-15);
}

#[test]
fn float_with_plus_exponent() {
    let (v, _) = as_float(&single_clean("2.5E+2"));
    assert!((v - 250.0).abs() < 1e-9);
}

#[test]
fn float_dot_then_exponent() {
    // `1.e5` is a decimal float — 100000.0.
    let (v, _) = as_float(&single_clean("1.e5"));
    assert!((v - 1e5).abs() < 1.0);
}

#[test]
fn float_dotless_exponent_only() {
    let (v, _) = as_float(&single_clean("3E4"));
    assert!((v - 3e4).abs() < 1.0);
}

#[test]
fn float_exponent_without_digits_is_error() {
    let (_, diags) = lex_with_diags("1e");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("exponent has no digits")),
        "expected exponent diagnostic, got {diags:?}"
    );
}

// ---------- Float suffixes ----------

#[test]
fn float_suffix_f() {
    let (_, s) = as_float(&single_clean("1.5f"));
    assert_eq!(s, FloatSuffix::F);
    let (_, s) = as_float(&single_clean("1.5F"));
    assert_eq!(s, FloatSuffix::F);
}

#[test]
fn float_suffix_l() {
    let (_, s) = as_float(&single_clean("1.5l"));
    assert_eq!(s, FloatSuffix::L);
    let (_, s) = as_float(&single_clean("1.5L"));
    assert_eq!(s, FloatSuffix::L);
}

#[test]
fn float_suffix_on_leading_dot() {
    let (v, s) = as_float(&single_clean(".25f"));
    assert!((v - 0.25).abs() < 1e-12);
    assert_eq!(s, FloatSuffix::F);
}

#[test]
fn float_suffix_on_exponent_form() {
    let (v, s) = as_float(&single_clean("1e2F"));
    assert!((v - 100.0).abs() < 1e-9);
    assert_eq!(s, FloatSuffix::F);
}

// =====================================================================
// Hex floats
// =====================================================================

#[test]
fn hex_float_simple() {
    // 0x1.8p1 = (1 + 8/16) * 2^1 = 1.5 * 2 = 3.0
    let (v, _) = as_float(&single_clean("0x1.8p1"));
    assert!((v - 3.0).abs() < 1e-12);
}

#[test]
fn hex_float_no_fractional_part() {
    // 0x1p3 = 1 * 8 = 8.0
    let (v, _) = as_float(&single_clean("0x1p3"));
    assert!((v - 8.0).abs() < 1e-12);
}

#[test]
fn hex_float_no_integer_part() {
    // 0x.8p2 = 0.5 * 4 = 2.0
    let (v, _) = as_float(&single_clean("0x.8p2"));
    assert!((v - 2.0).abs() < 1e-12);
}

#[test]
fn hex_float_negative_exponent() {
    // 0x1p-1 = 1 * 0.5 = 0.5
    let (v, _) = as_float(&single_clean("0x1p-1"));
    assert!((v - 0.5).abs() < 1e-12);
}

#[test]
fn hex_float_uppercase_p() {
    let (v, _) = as_float(&single_clean("0x1P3"));
    assert!((v - 8.0).abs() < 1e-12);
}

#[test]
fn hex_float_with_suffix() {
    let (v, s) = as_float(&single_clean("0x1.8p1f"));
    assert!((v - 3.0).abs() < 1e-12);
    assert_eq!(s, FloatSuffix::F);
}

#[test]
fn hex_float_missing_binary_exponent_is_error() {
    let (_, diags) = lex_with_diags("0x1.5");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("missing binary exponent")),
        "expected binary-exponent diagnostic, got {diags:?}"
    );
}

#[test]
fn hex_float_p_without_digits_is_error() {
    let (_, diags) = lex_with_diags("0x1.5p");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("exponent has no digits")),
        "expected exponent-no-digits diagnostic, got {diags:?}"
    );
}

// =====================================================================
// Integer / float boundaries that produce a float
// =====================================================================

#[test]
fn int_then_dot_at_eof_is_float() {
    // `1.` alone — treated as float 1.0 per task spec.
    let (toks, diags) = lex_with_diags("1.");
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 1);
    let (v, _) = as_float(&toks[0].kind);
    assert!((v - 1.0).abs() < 1e-12);
}

#[test]
fn double_dot_after_integer_is_float_then_dot() {
    // `1..` — `1.` is a float (no identifier after dot), then `.`.
    let (toks, diags) = lex_with_diags("1..");
    assert!(diags.is_empty());
    assert_eq!(toks.len(), 2);
    let (v, _) = as_float(&toks[0].kind);
    assert!((v - 1.0).abs() < 1e-12);
    assert_eq!(toks[1].kind, TokenKind::Dot);
}

#[test]
fn float_then_semicolon() {
    let (toks, _) = lex_with_diags("1.5;");
    assert_eq!(toks.len(), 2);
    assert!(matches!(toks[0].kind, TokenKind::FloatLiteral { .. }));
    assert_eq!(toks[1].kind, TokenKind::Semicolon);
}

// =====================================================================
// Spans
// =====================================================================

#[test]
fn float_literal_span_covers_value_and_suffix() {
    let (toks, _) = lex_with_diags("1.5e-3f");
    assert_eq!(toks[0].span, Span::new(0, 7));
}

#[test]
fn float_from_leading_dot_span() {
    let (toks, _) = lex_with_diags(".5");
    assert_eq!(toks[0].span, Span::new(0, 2));
}

// =====================================================================
// Extra edge cases flagged while closing out phase 1.2
// =====================================================================

#[test]
fn hex_float_with_l_suffix() {
    // Parallel to `hex_float_with_suffix` but exercising the `l` path.
    let (v, s) = as_float(&single_clean("0x1.8p1L"));
    assert!((v - 3.0).abs() < 1e-12);
    assert_eq!(s, FloatSuffix::L);
}

#[test]
fn huge_decimal_exponent_becomes_infinity() {
    // f64 maxes out around 1e308; `1e9999` overflows to +inf.
    // This path exercises f64::parse() returning Ok(inf) — not an
    // error — so no diagnostic is emitted.  The lexer faithfully
    // hands the infinite value to later phases.
    let (toks, diags) = lex_with_diags("1e9999");
    assert!(diags.is_empty(), "no diagnostic expected: {diags:?}");
    assert_eq!(toks.len(), 1);
    let (v, _) = as_float(&toks[0].kind);
    assert!(v.is_infinite() && v.is_sign_positive());
}

#[test]
fn hex_float_leading_dot_with_empty_integer_part() {
    // `0x.` (no hex digits on either side of the dot) emits the
    // "no hex digits" error and recovers as a 0.0 float.
    let (toks, diags) = lex_with_diags("0x.p0");
    assert!(
        diags.iter().any(|d| d.message.contains("no hex digits")),
        "expected `no hex digits` diagnostic, got {diags:?}"
    );
    // We still produce a float token so downstream phases don't choke.
    assert_eq!(toks.len(), 1);
    assert!(matches!(toks[0].kind, TokenKind::FloatLiteral { .. }));
}

#[test]
fn zero_point_zero_is_float_not_octal() {
    // `0.0` must route through the float path (dot + digit after zero
    // trumps octal interpretation).
    let (v, s) = as_float(&single_clean("0.0"));
    assert_eq!(v, 0.0);
    assert_eq!(s, FloatSuffix::None);
}

#[test]
fn zero_exponent_is_float_not_octal() {
    // `0e5` must route through the float path (exponent trumps octal).
    let (v, _) = as_float(&single_clean("0e5"));
    assert_eq!(v, 0.0);
}

#[test]
fn hex_literal_followed_by_dot_identifier_eats_dot() {
    // The documented trade-off for hex: `0x1.method` will be treated as
    // the start of a (malformed) hex float, NOT hex 0x1 + `.method`.
    // This test pins that behaviour so we notice if it ever changes.
    let (toks, diags) = lex_with_diags("0x1.method");
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("missing binary exponent")),
        "expected missing-exponent diagnostic, got {diags:?}"
    );
    // First token is a recovered hex float; then the `method` identifier.
    assert!(matches!(toks[0].kind, TokenKind::FloatLiteral { .. }));
    assert_eq!(toks[1].kind, TokenKind::Identifier("method".to_string()));
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
