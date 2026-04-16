//! Tests covering the `##` (token-paste) operator and `paste_spelling`.

use forge_diagnostics::Severity;
use forge_lexer::{Lexer, TokenKind};

use super::helpers::*;
use crate::expand::paste_spelling;

// -----------------------------------------------------------------
// paste_spelling helper
// -----------------------------------------------------------------

#[test]
fn paste_spelling_concatenates_both_sides() {
    let left = Lexer::new("foo").tokenize().into_iter().next().unwrap();
    let right = Lexer::new("bar").tokenize().into_iter().next().unwrap();
    assert_eq!(paste_spelling(Some(&left), Some(&right)), "foobar");
}

#[test]
fn paste_spelling_handles_empty_left_or_right() {
    let only = Lexer::new("foo").tokenize().into_iter().next().unwrap();
    assert_eq!(paste_spelling(Some(&only), None), "foo");
    assert_eq!(paste_spelling(None, Some(&only)), "foo");
    assert_eq!(paste_spelling(None, None), "");
}

// -----------------------------------------------------------------
// Token pasting — §6.10.3.3
// -----------------------------------------------------------------

#[test]
fn paste_two_identifiers_into_a_single_identifier() {
    // PASTE(foo, bar) → foobar
    let (mut pp, out) = run("#define PASTE(a, b) a##b\nPASTE(foo, bar)");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: foobar, Eof
    assert_eq!(ks.len(), 2);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "foobar"));
    assert!(matches!(ks[1], TokenKind::Eof));
}

#[test]
fn paste_identifier_with_number_yields_suffixed_identifier() {
    // PASTE(x, 3) → x3
    let (mut pp, out) = run("#define PASTE(a, b) a##b\nPASTE(x, 3)");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    assert_eq!(ks.len(), 2);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "x3"));
}

#[test]
fn paste_number_with_number_yields_single_integer_literal() {
    // PASTE(1, 2) → 12
    let (mut pp, out) = run("#define PASTE(a, b) a##b\nPASTE(1, 2)");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    assert_eq!(ks.len(), 2);
    assert!(matches!(ks[0], TokenKind::IntegerLiteral { value: 12, .. }));
}

#[test]
fn paste_uses_raw_argument_without_pre_expansion() {
    // With #define N 5 and #define CAT(a,b) a##b:
    // CAT(N, 1) → N1 (identifier), not 51.  Parameters adjacent to
    // `##` use the raw argument tokens.
    let (mut pp, out) = run("#define N 5\n#define CAT(a, b) a##b\nCAT(N, 1)");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    assert_eq!(ks.len(), 2);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "N1"));
}

#[test]
fn paste_placeholder_left_side_yields_right_side_alone() {
    // CAT(, foo) → foo (empty left side is the "placeholder").
    let (mut pp, out) = run("#define CAT(a, b) a##b\nCAT(, foo)");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    assert_eq!(ks.len(), 2);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "foo"));
}

#[test]
fn paste_placeholder_right_side_yields_left_side_alone() {
    // CAT(foo, ) → foo (empty right side).
    let (mut pp, out) = run("#define CAT(a, b) a##b\nCAT(foo, )");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    assert_eq!(ks.len(), 2);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "foo"));
}

#[test]
fn paste_of_two_placeholders_produces_no_tokens() {
    // CAT(,) with a plain `a##b` body collapses to nothing; the
    // surrounding punctuators survive.
    let (mut pp, out) = run("#define CAT(a, b) [a##b]\nCAT(,)");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: [, ], Eof
    assert_eq!(ks.len(), 3);
    assert!(matches!(ks[0], TokenKind::LeftBracket));
    assert!(matches!(ks[1], TokenKind::RightBracket));
    assert!(matches!(ks[2], TokenKind::Eof));
}

#[test]
fn paste_result_that_matches_a_macro_name_is_rescanned() {
    // PASTE(fo, o) builds the identifier `foo`, which is itself a
    // macro and must expand on rescan.
    let (mut pp, out) = run("#define foo 42\n#define PASTE(a, b) a##b\nPASTE(fo, o)");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    assert_eq!(ks.len(), 2);
    assert!(matches!(ks[0], TokenKind::IntegerLiteral { value: 42, .. }));
}

#[test]
fn paste_invalid_combination_emits_a_warning_but_keeps_tokens() {
    // `+ ;` is not a single preprocessing token — a warning fires
    // but both tokens survive.
    let (mut pp, out) = run("#define CAT(a, b) a##b\nCAT(+, ;)");
    let diags = pp.take_diagnostics();
    assert!(
        diags
            .iter()
            .any(|d| matches!(d.severity, Severity::Warning)),
        "expected a warning, got {diags:?}"
    );
    let ks = kinds_of(&out);
    // Expected: +, ;, Eof
    assert_eq!(ks.len(), 3);
    assert!(matches!(ks[0], TokenKind::Plus));
    assert!(matches!(ks[1], TokenKind::Semicolon));
}

#[test]
fn stringify_followed_by_paste_composes_correctly() {
    // Combine # and ##: STR_CAT(a, b) → "ab" via stringifying a
    // paste of its two raw operands.  Actually simpler: verify that
    // a macro that uses both # and ## in the same body works.
    //   #define NAMED(pre, x) pre##_##x = #x
    //   NAMED(var, hi)  →  var_hi = "hi"
    let (mut pp, out) = run("#define NAMED(pre, x) pre##_##x = #x\nNAMED(var, hi);");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: var_hi, =, "hi", ;, Eof
    assert_eq!(ks.len(), 5);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "var_hi"));
    assert!(matches!(ks[1], TokenKind::Equal));
    assert!(matches!(ks[2], TokenKind::StringLiteral { ref value, .. } if value == "hi"));
    assert!(matches!(ks[3], TokenKind::Semicolon));
}
