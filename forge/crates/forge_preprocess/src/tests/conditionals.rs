//! `#if`/`#ifdef`/`#elif`/`#else` flow plus `cond_expr::evaluate` behaviour.

use forge_diagnostics::{Diagnostic, Severity};
use forge_lexer::{Lexer, Span, Token, TokenKind};

use super::helpers::*;
use crate::cond_expr::{evaluate, PPValue};

// -----------------------------------------------------------------
// Conditional compilation — §6.10.1
// -----------------------------------------------------------------

#[test]
fn ifdef_emits_body_when_the_name_is_defined() {
    let (mut pp, out) = run("#define FOO\n#ifdef FOO\nYES\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn ifdef_skips_body_when_the_name_is_not_defined() {
    let (mut pp, out) = run("#ifdef NOT_DEFINED\nNO\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert!(identifier_names(&out).is_empty());
}

#[test]
fn ifndef_is_the_logical_inverse_of_ifdef() {
    let (mut pp1, out1) = run("#ifndef NOT_DEFINED\nYES\n#endif\n");
    assert!(no_errors(&pp1.take_diagnostics()));
    assert_eq!(identifier_names(&out1), vec!["YES"]);

    let (mut pp2, out2) = run("#define FOO\n#ifndef FOO\nNO\n#endif\n");
    assert!(no_errors(&pp2.take_diagnostics()));
    assert!(identifier_names(&out2).is_empty());
}

#[test]
fn if_literal_one_is_active_and_if_literal_zero_is_inactive() {
    let (mut pp1, out1) = run("#if 1\nYES\n#endif\n");
    assert!(no_errors(&pp1.take_diagnostics()));
    assert_eq!(identifier_names(&out1), vec!["YES"]);

    let (mut pp2, out2) = run("#if 0\nNO\n#endif\n");
    assert!(no_errors(&pp2.take_diagnostics()));
    assert!(identifier_names(&out2).is_empty());
}

#[test]
fn if_arithmetic_expression_non_zero_is_active() {
    // 1 + 1 → 2 → active.
    let (mut pp, out) = run("#if 1 + 1\nYES\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn if_defined_with_parens_is_active_when_name_is_defined() {
    let (mut pp, out) = run("#define FOO\n#if defined(FOO)\nYES\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn if_defined_without_parens_is_also_valid_syntax() {
    let (mut pp, out) = run("#define FOO\n#if defined FOO\nYES\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn if_defined_and_defined_requires_both_names_defined() {
    let src_both = "#define FOO\n#define BAR\n\
                    #if defined(FOO) && defined(BAR)\nYES\n#endif\n";
    let (mut pp, out) = run(src_both);
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);

    let src_one = "#define FOO\n#if defined(FOO) && defined(BAR)\nYES\n#endif\n";
    let (mut pp2, out2) = run(src_one);
    assert!(no_errors(&pp2.take_diagnostics()));
    assert!(identifier_names(&out2).is_empty());
}

#[test]
fn if_expression_sees_macros_after_expansion() {
    // FOO expands to 42, so the comparison holds.
    let (mut pp, out) = run("#define FOO 42\n#if FOO == 42\nYES\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn undefined_identifier_in_if_evaluates_to_zero() {
    // `UNKNOWN` is not a macro, so it becomes 0 and `0 == 0` is true.
    let (mut pp, out) = run("#if UNKNOWN == 0\nYES\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn elif_chain_first_true_branch_wins() {
    let src = "#if 0\nA\n#elif 1\nB\n#elif 1\nC\n#else\nD\n#endif\n";
    let (mut pp, out) = run(src);
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["B"]);
}

#[test]
fn elif_chain_with_all_branches_false_falls_to_else() {
    let src = "#if 0\nA\n#elif 0\nB\n#else\nC\n#endif\n";
    let (mut pp, out) = run(src);
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["C"]);
}

#[test]
fn else_is_inactive_when_an_earlier_branch_was_taken() {
    let src = "#if 1\nA\n#else\nB\n#endif\n";
    let (mut pp, out) = run(src);
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["A"]);
}

#[test]
fn nested_if_inside_if_one_both_inner_and_outer_active() {
    let src = "#if 1\nA\n#if 1\nB\n#endif\nC\n#endif\n";
    let (mut pp, out) = run(src);
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["A", "B", "C"]);
}

#[test]
fn nested_if_zero_inside_if_one_inner_inactive_outer_active() {
    let src = "#if 1\nA\n#if 0\nB\n#endif\nC\n#endif\n";
    let (mut pp, out) = run(src);
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["A", "C"]);
}

#[test]
fn if_zero_skips_arbitrary_junk_without_errors() {
    // `"unterminated string` and `#not_a_directive` inside `#if 0`
    // must not produce diagnostics — the group is skipped
    // structurally only.
    let src = "#if 0\nstuff \"unterminated\nand #not_a_directive here\n#endif\nreal";
    let (mut pp, out) = run(src);
    let diags = pp.take_diagnostics();
    assert!(
        diags.iter().all(|d| !matches!(d.severity, Severity::Error)),
        "#if 0 should not error on malformed inner content: {diags:?}"
    );
    assert_eq!(identifier_names(&out), vec!["real"]);
}

#[test]
fn else_without_matching_if_is_an_error() {
    let (mut pp, _) = run("#else\n#endif\n");
    let diags = pp.take_diagnostics();
    assert!(
        diags
            .iter()
            .any(|d| matches!(d.severity, Severity::Error) && d.message.contains("`#else`")),
        "expected `#else` without matching `#if` error, got {diags:?}"
    );
}

#[test]
fn endif_without_matching_if_is_an_error() {
    let (mut pp, _) = run("#endif\n");
    let diags = pp.take_diagnostics();
    assert!(
        diags
            .iter()
            .any(|d| matches!(d.severity, Severity::Error) && d.message.contains("`#endif`")),
        "expected unmatched `#endif` error, got {diags:?}"
    );
}

#[test]
fn elif_after_else_is_an_error() {
    let src = "#if 0\n#else\n#elif 1\n#endif\n";
    let (mut pp, _) = run(src);
    let diags = pp.take_diagnostics();
    assert!(
        diags
            .iter()
            .any(|d| matches!(d.severity, Severity::Error) && d.message.contains("`#elif`")),
        "expected `#elif after #else` error, got {diags:?}"
    );
}

#[test]
fn duplicate_else_in_same_if_block_is_an_error() {
    let src = "#if 0\n#else\n#else\n#endif\n";
    let (mut pp, _) = run(src);
    let diags = pp.take_diagnostics();
    assert!(
        diags
            .iter()
            .any(|d| matches!(d.severity, Severity::Error) && d.message.contains("`#else`")),
        "expected duplicate `#else` error, got {diags:?}"
    );
}

#[test]
fn unterminated_if_at_end_of_file_is_an_error() {
    let (mut pp, _) = run("#if 1\nabc\n");
    let diags = pp.take_diagnostics();
    assert!(
        diags
            .iter()
            .any(|d| matches!(d.severity, Severity::Error) && d.message.contains("unterminated")),
        "expected unterminated-`#if` error, got {diags:?}"
    );
}

#[test]
fn if_character_literal_in_expression() {
    let (mut pp, out) = run("#if 'A' == 65\nYES\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn if_shift_expression() {
    let (mut pp, out) = run("#if (1 << 4) == 16\nYES\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn if_logical_or_short_circuits_to_true() {
    let (mut pp, out) = run("#if 0 || 1\nYES\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn if_signed_minus_one_is_promoted_when_compared_to_unsigned_literal() {
    // `-1` becomes UINTMAX_MAX under the usual arithmetic
    // conversions, which is NOT less than `1U` — this branch must
    // be inactive.
    let (mut pp, out) = run("#if -1 < 1U\nYES\n#else\nNO\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["NO"]);
}

#[test]
fn if_unsigned_wrapping_subtraction_produces_max_value() {
    // `0U - 1` wraps to UINTMAX_MAX, which is > 0 — active.
    let (mut pp, out) = run("#if 0U - 1 > 0\nYES\n#else\nNO\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn if_unsigned_long_long_arithmetic_preserves_tag() {
    let (mut pp, out) = run("#if 1ULL + 1ULL == 2\nYES\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn if_combined_expression_with_defined_and_logic() {
    // Simulates `(defined(__linux__) && defined(__x86_64__)) ||
    // defined(__aarch64__)`.  We stand in for the first two being
    // absent and the third being defined — result must be active.
    let src = "#define __aarch64__\n\
               #if (defined(__linux__) && defined(__x86_64__)) || defined(__aarch64__)\n\
               YES\n\
               #endif\n";
    let (mut pp, out) = run(src);
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn if_defined_uses_raw_name_not_its_expansion() {
    // `FOO` is a macro whose replacement is another identifier —
    // `defined(FOO)` must still see the name `FOO`, not expand it.
    let (mut pp, out) = run("#define FOO BAR\n#if defined(FOO)\nYES\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn nested_if_one_inside_if_zero_stays_inactive() {
    // The outer `#if 0` skips its body.  The inner `#if 1` still
    // opens and closes its own frame but cannot emit anything.
    let src = "#if 0\n#if 1\nBAD\n#endif\n#endif\nOK\n";
    let (mut pp, out) = run(src);
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["OK"]);
}

#[test]
fn elif_expression_is_not_evaluated_when_inside_skipped_group() {
    // The outer `#if 0` is inactive.  The inner `#elif 1 / 0`
    // would warn about division by zero if evaluated, but must
    // not be evaluated because the enclosing frame is inactive.
    let src = "#if 0\n#if 0\n#elif 1 / 0\nX\n#endif\n#endif\n";
    let (mut pp, _) = run(src);
    let diags = pp.take_diagnostics();
    assert!(
        diags
            .iter()
            .all(|d| !matches!(d.severity, Severity::Warning | Severity::Error)),
        "expression in a skipped group must not be evaluated: {diags:?}"
    );
}

// -----------------------------------------------------------------
// cond_expr::evaluate — direct expression-evaluator tests
// -----------------------------------------------------------------

fn lex_cond(src: &str) -> Vec<Token> {
    let mut toks = Lexer::new(src).tokenize();
    // Drop the trailing Eof so expression-parser tests see a clean
    // stream; the parser also treats the absent sentinel as
    // end-of-input.
    if matches!(toks.last().map(|t| &t.kind), Some(TokenKind::Eof)) {
        toks.pop();
    }
    toks
}

fn eval(src: &str) -> (PPValue, Vec<Diagnostic>) {
    let tokens = lex_cond(src);
    evaluate(&tokens, Span::new(0, 0))
}

fn eval_value(src: &str) -> PPValue {
    let (v, d) = eval(src);
    assert!(
        d.iter().all(|x| !matches!(x.severity, Severity::Error)),
        "unexpected errors for `{src}`: {d:?}"
    );
    v
}

#[test]
fn integer_literal_no_suffix_is_signed() {
    assert_eq!(eval_value("42"), PPValue::Signed(42));
}

#[test]
fn integer_literal_u_suffix_is_unsigned() {
    assert_eq!(eval_value("42U"), PPValue::Unsigned(42));
}

#[test]
fn integer_literal_ull_suffix_is_unsigned() {
    assert_eq!(eval_value("7ULL"), PPValue::Unsigned(7));
}

#[test]
fn character_literal_is_signed() {
    assert_eq!(eval_value("'A'"), PPValue::Signed(65));
}

#[test]
fn basic_arithmetic() {
    assert_eq!(eval_value("1 + 2"), PPValue::Signed(3));
    assert_eq!(eval_value("10 - 3"), PPValue::Signed(7));
    assert_eq!(eval_value("4 * 5"), PPValue::Signed(20));
    assert_eq!(eval_value("20 / 3"), PPValue::Signed(6));
    assert_eq!(eval_value("20 % 3"), PPValue::Signed(2));
}

#[test]
fn precedence_times_over_plus() {
    assert_eq!(eval_value("1 + 2 * 3"), PPValue::Signed(7));
    assert_eq!(eval_value("(1 + 2) * 3"), PPValue::Signed(9));
}

#[test]
fn shift_operators() {
    assert_eq!(eval_value("1 << 4"), PPValue::Signed(16));
    assert_eq!(eval_value("256 >> 4"), PPValue::Signed(16));
}

#[test]
fn comparison_operators_return_signed_bool() {
    assert_eq!(eval_value("1 < 2"), PPValue::Signed(1));
    assert_eq!(eval_value("2 < 1"), PPValue::Signed(0));
    assert_eq!(eval_value("3 == 3"), PPValue::Signed(1));
    assert_eq!(eval_value("3 != 3"), PPValue::Signed(0));
}

#[test]
fn logical_and_short_circuits() {
    // `1 / 0` would warn if evaluated; in `&&` with a false LHS it
    // must be skipped entirely.
    let (v, d) = eval("0 && (1 / 0)");
    assert_eq!(v, PPValue::Signed(0));
    assert!(
        d.iter()
            .all(|x| !matches!(x.severity, Severity::Error | Severity::Warning)),
        "expected no division-by-zero warning, got {d:?}"
    );
}

#[test]
fn logical_or_short_circuits() {
    let (v, d) = eval("1 || (1 / 0)");
    assert_eq!(v, PPValue::Signed(1));
    assert!(
        d.iter()
            .all(|x| !matches!(x.severity, Severity::Error | Severity::Warning)),
        "expected no division-by-zero warning, got {d:?}"
    );
}

#[test]
fn unsigned_promotes_signed_in_comparison() {
    // The textbook trap: `-1 < 1U` is false because -1 is
    // converted to unsigned first.
    assert_eq!(eval_value("-1 < 1U"), PPValue::Signed(0));
}

#[test]
fn unsigned_subtraction_wraps_to_max() {
    // 0U - 1 = UINT64_MAX, which is > 0.
    assert_eq!(eval_value("0U - 1 > 0"), PPValue::Signed(1));
}

#[test]
fn unsigned_addition_preserves_unsigned_tag() {
    assert_eq!(eval_value("1ULL + 1ULL"), PPValue::Unsigned(2));
}

#[test]
fn unary_minus_on_signed_is_two_complement() {
    assert_eq!(eval_value("-5"), PPValue::Signed(-5));
    assert_eq!(eval_value("-(3 + 2)"), PPValue::Signed(-5));
}

#[test]
fn ternary_selects_taken_branch_only() {
    assert_eq!(eval_value("1 ? 10 : 20"), PPValue::Signed(10));
    assert_eq!(eval_value("0 ? 10 : 20"), PPValue::Signed(20));
}

#[test]
fn division_by_zero_emits_warning_and_returns_zero() {
    let (v, d) = eval("1 / 0");
    assert_eq!(v, PPValue::Signed(0));
    assert!(d.iter().any(|x| matches!(x.severity, Severity::Warning)));
}

#[test]
fn unexpected_trailing_tokens_emit_error() {
    let (_, d) = eval("1 2");
    assert!(d.iter().any(|x| matches!(x.severity, Severity::Error)));
}
