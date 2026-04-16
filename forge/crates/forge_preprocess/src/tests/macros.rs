//! `#define`/`#undef` storage plus object- and function-like macro expansion.

use forge_diagnostics::Severity;
use forge_lexer::TokenKind;

use super::helpers::*;
use crate::MacroDef;

// -----------------------------------------------------------------
// #define — storage
// -----------------------------------------------------------------

#[test]
fn define_stores_an_object_like_macro() {
    let (pp, _) = run("#define FOO 42\n");
    let m = pp.macros().get("FOO").expect("FOO should be stored");
    match m {
        MacroDef::ObjectLike {
            name,
            replacement,
            is_predefined,
        } => {
            assert_eq!(name, "FOO");
            assert!(!is_predefined);
            assert_eq!(replacement.len(), 1);
            assert!(matches!(
                replacement[0].kind,
                TokenKind::IntegerLiteral { value: 42, .. }
            ));
        }
        other => panic!("expected ObjectLike, got {other:?}"),
    }
}

#[test]
fn define_empty_body_stores_an_empty_object_like_macro() {
    let (pp, _) = run("#define FLAG\n");
    let m = pp.macros().get("FLAG").expect("FLAG should be stored");
    match m {
        MacroDef::ObjectLike { replacement, .. } => {
            assert!(replacement.is_empty());
        }
        other => panic!("expected ObjectLike, got {other:?}"),
    }
}

#[test]
fn define_stores_a_function_like_macro_when_paren_has_no_leading_space() {
    let (pp, _) = run("#define ADD(a, b) a + b\n");
    let m = pp.macros().get("ADD").expect("ADD should be stored");
    match m {
        MacroDef::FunctionLike {
            name,
            params,
            is_variadic,
            replacement,
        } => {
            assert_eq!(name, "ADD");
            assert_eq!(params, &vec!["a".to_string(), "b".to_string()]);
            assert!(!is_variadic);
            // Replacement tokens: a, +, b
            assert_eq!(replacement.len(), 3);
        }
        other => panic!("expected FunctionLike, got {other:?}"),
    }
}

#[test]
fn define_with_space_before_paren_is_object_like_not_function_like() {
    // The `(` has a leading space, so it is part of the replacement
    // list — the macro is object-like with replacement `(x) x`.
    let (pp, _) = run("#define F (x) x\n");
    let m = pp.macros().get("F").expect("F should be stored");
    match m {
        MacroDef::ObjectLike { replacement, .. } => {
            // Replacement is: (, x, ), x
            assert_eq!(replacement.len(), 4);
            assert!(matches!(replacement[0].kind, TokenKind::LeftParen));
            assert!(matches!(replacement[3].kind, TokenKind::Identifier(ref s) if s == "x"));
        }
        other => panic!("expected ObjectLike, got {other:?}"),
    }
}

#[test]
fn define_function_like_with_no_params_stores_empty_param_list() {
    let (pp, _) = run("#define NOW() 12345\n");
    let m = pp.macros().get("NOW").expect("NOW should be stored");
    match m {
        MacroDef::FunctionLike {
            params,
            is_variadic,
            replacement,
            ..
        } => {
            assert!(params.is_empty());
            assert!(!is_variadic);
            assert_eq!(replacement.len(), 1);
        }
        other => panic!("expected FunctionLike, got {other:?}"),
    }
}

#[test]
fn define_variadic_macro_sets_is_variadic() {
    let (pp, _) = run("#define LOG(fmt, ...) printf(fmt, __VA_ARGS__)\n");
    let m = pp.macros().get("LOG").expect("LOG should be stored");
    match m {
        MacroDef::FunctionLike {
            params,
            is_variadic,
            ..
        } => {
            assert_eq!(params, &vec!["fmt".to_string()]);
            assert!(is_variadic);
        }
        other => panic!("expected FunctionLike, got {other:?}"),
    }
}

// -----------------------------------------------------------------
// #undef
// -----------------------------------------------------------------

#[test]
fn undef_removes_a_defined_macro() {
    let (mut pp, _) = run("#define FOO 42\n#undef FOO\n");
    assert!(pp.macros().get("FOO").is_none());
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
}

#[test]
fn undef_of_undefined_macro_is_silently_allowed() {
    let (mut pp, _) = run("#undef NEVER_DEFINED\n");
    let diags = pp.take_diagnostics();
    assert!(
        !diags.iter().any(|d| matches!(d.severity, Severity::Error)),
        "unexpected errors: {diags:?}"
    );
    assert!(pp.macros().get("NEVER_DEFINED").is_none());
}

// -----------------------------------------------------------------
// Redefinition
// -----------------------------------------------------------------

#[test]
fn redefining_with_equivalent_replacement_emits_no_diagnostic() {
    let (mut pp, _) = run("#define X 1 + 2\n#define X 1 + 2\n");
    let diags = pp.take_diagnostics();
    assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
}

#[test]
fn redefining_ignores_whitespace_amount() {
    let (mut pp, _) = run("#define X 1 + 2\n#define X 1    +   2\n");
    let diags = pp.take_diagnostics();
    assert!(
        diags.is_empty(),
        "expected no diagnostics for whitespace-only difference, got {diags:?}"
    );
}

#[test]
fn redefining_with_different_replacement_warns() {
    let (mut pp, _) = run("#define X 1\n#define X 2\n");
    let diags = pp.take_diagnostics();
    assert_eq!(diags.len(), 1, "expected exactly one diagnostic");
    assert!(matches!(diags[0].severity, Severity::Warning));
    assert!(
        diags[0].message.contains("`X` redefined"),
        "unexpected message: {}",
        diags[0].message
    );
}

#[test]
fn redefining_object_like_as_function_like_warns() {
    let (mut pp, _) = run("#define F 1\n#define F(x) x\n");
    let diags = pp.take_diagnostics();
    assert_eq!(diags.len(), 1);
    assert!(matches!(diags[0].severity, Severity::Warning));
}

// -----------------------------------------------------------------
// Object-like macro expansion
// -----------------------------------------------------------------

#[test]
fn object_like_macro_is_expanded_in_place() {
    // `N` must be replaced by `42` in the output stream.
    let (mut pp, out) = run("#define N 42\nint x = N;");
    assert!(pp.take_diagnostics().is_empty(), "expected no diagnostics");
    let ks = kinds_of(&out);
    // Expected: int, x, =, 42, ;, Eof
    assert_eq!(ks.len(), 6);
    assert!(matches!(ks[0], TokenKind::Int));
    assert!(matches!(ks[1], TokenKind::Identifier(ref s) if s == "x"));
    assert!(matches!(ks[2], TokenKind::Equal));
    assert!(matches!(ks[3], TokenKind::IntegerLiteral { value: 42, .. }));
    assert!(matches!(ks[4], TokenKind::Semicolon));
    assert!(matches!(ks[5], TokenKind::Eof));
}

#[test]
fn macro_chain_expansion_rescans_the_replacement() {
    // A → B → 42.  The intermediate identifier B must itself be
    // expanded during the rescan.
    let (_, out) = run("#define A B\n#define B 42\nA");
    let ks = kinds_of(&out);
    // Expected: 42, Eof
    assert_eq!(ks.len(), 2);
    assert!(matches!(ks[0], TokenKind::IntegerLiteral { value: 42, .. }));
    assert!(matches!(ks[1], TokenKind::Eof));
}

#[test]
fn self_referential_macro_terminates_and_emits_the_name_once() {
    // `#define X X` must not loop forever — the hide set stops
    // the rescan from re-entering X.
    let (_, out) = run("#define X X\nX");
    let ks = kinds_of(&out);
    // Expected: X, Eof
    assert_eq!(ks.len(), 2);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "X"));
    assert!(matches!(ks[1], TokenKind::Eof));
}

#[test]
fn mutually_recursive_macros_terminate_at_the_origin_name() {
    // A → B → A, but the second A carries {A, B} in its hide set so
    // the rescan stops and emits A.
    let (_, out) = run("#define A B\n#define B A\nA");
    let ks = kinds_of(&out);
    assert_eq!(ks.len(), 2);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "A"));
    assert!(matches!(ks[1], TokenKind::Eof));
}

#[test]
fn multi_token_replacement_emits_all_replacement_tokens() {
    // PI → `3 14` (two tokens).
    let (_, out) = run("#define PI 3 14\nPI");
    let ks = kinds_of(&out);
    // Expected: 3, 14, Eof
    assert_eq!(ks.len(), 3);
    assert!(matches!(ks[0], TokenKind::IntegerLiteral { value: 3, .. }));
    assert!(matches!(ks[1], TokenKind::IntegerLiteral { value: 14, .. }));
    assert!(matches!(ks[2], TokenKind::Eof));
}

#[test]
fn empty_macro_vanishes_without_leaving_a_trace() {
    // EMPTY has no replacement list — the invocation must disappear
    // entirely from the output, leaving the surrounding tokens
    // intact.
    let (_, out) = run("#define EMPTY\nint EMPTY x;");
    let ks = kinds_of(&out);
    // Expected: int, x, ;, Eof
    assert_eq!(ks.len(), 4);
    assert!(matches!(ks[0], TokenKind::Int));
    assert!(matches!(ks[1], TokenKind::Identifier(ref s) if s == "x"));
    assert!(matches!(ks[2], TokenKind::Semicolon));
    assert!(matches!(ks[3], TokenKind::Eof));
}

#[test]
fn macro_expansion_preserves_surrounding_tokens() {
    // The expansion must splice into the middle of the stream
    // without disturbing neighbours.
    let (_, out) = run("#define N 42\nint x = N * 2;");
    let ks = kinds_of(&out);
    // Expected: int, x, =, 42, *, 2, ;, Eof
    assert_eq!(ks.len(), 8);
    assert!(matches!(ks[0], TokenKind::Int));
    assert!(matches!(ks[1], TokenKind::Identifier(ref s) if s == "x"));
    assert!(matches!(ks[2], TokenKind::Equal));
    assert!(matches!(ks[3], TokenKind::IntegerLiteral { value: 42, .. }));
    assert!(matches!(ks[4], TokenKind::Star));
    assert!(matches!(ks[5], TokenKind::IntegerLiteral { value: 2, .. }));
    assert!(matches!(ks[6], TokenKind::Semicolon));
    assert!(matches!(ks[7], TokenKind::Eof));
}

#[test]
fn function_like_macro_without_invocation_stays_unexpanded() {
    // `F` alone — with no following `(` — is not a function-like
    // macro invocation.  Object-like expansion must leave it alone,
    // and function-like expansion (not implemented yet) also must
    // not fire.
    let (_, out) = run("#define F(x) x\nF;");
    let ks = kinds_of(&out);
    // Expected: F, ;, Eof
    assert_eq!(ks.len(), 3);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "F"));
    assert!(matches!(ks[1], TokenKind::Semicolon));
    assert!(matches!(ks[2], TokenKind::Eof));
}

#[test]
fn undefined_identifier_passes_through_unchanged() {
    let (_, out) = run("foo");
    let ks = kinds_of(&out);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "foo"));
    assert!(matches!(ks[1], TokenKind::Eof));
}

#[test]
fn chained_expansion_propagates_hide_set_through_every_step() {
    // Three macros in a chain: A → B → C → 7.  At the last rescan,
    // the integer literal 7 is emitted and cannot match any macro
    // (it's not an identifier), so the chain terminates cleanly.
    let (_, out) = run("#define A B\n#define B C\n#define C 7\nA");
    let ks = kinds_of(&out);
    // Expected: 7, Eof
    assert_eq!(ks.len(), 2);
    assert!(matches!(ks[0], TokenKind::IntegerLiteral { value: 7, .. }));
    assert!(matches!(ks[1], TokenKind::Eof));
}

#[test]
fn macro_that_reintroduces_origin_is_blocked_by_hide_set() {
    // FOO → BAR FOO.  The second FOO comes from FOO's own
    // replacement and so inherits `{FOO}` in its hide set — so it
    // does not expand again.  Result: `BAR FOO ;`.
    let (_, out) = run("#define FOO BAR FOO\nFOO;");
    let ks = kinds_of(&out);
    // Expected: BAR, FOO, ;, Eof
    assert_eq!(ks.len(), 4);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "BAR"));
    assert!(matches!(ks[1], TokenKind::Identifier(ref s) if s == "FOO"));
    assert!(matches!(ks[2], TokenKind::Semicolon));
    assert!(matches!(ks[3], TokenKind::Eof));
}

// -----------------------------------------------------------------
// Function-like macro expansion — §6.10.3.1 … §6.10.3.4
// -----------------------------------------------------------------

#[test]
fn function_like_simple_single_param_is_substituted() {
    // SQUARE(5) → 5 * 5
    let (mut pp, out) = run("#define SQUARE(x) x * x\nSQUARE(5);");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: 5, *, 5, ;, Eof
    assert_eq!(ks.len(), 5);
    assert!(matches!(ks[0], TokenKind::IntegerLiteral { value: 5, .. }));
    assert!(matches!(ks[1], TokenKind::Star));
    assert!(matches!(ks[2], TokenKind::IntegerLiteral { value: 5, .. }));
    assert!(matches!(ks[3], TokenKind::Semicolon));
    assert!(matches!(ks[4], TokenKind::Eof));
}

#[test]
fn function_like_multi_param_in_order() {
    // ADD(1, 2) → 1 + 2
    let (mut pp, out) = run("#define ADD(a, b) a + b\nADD(1, 2);");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: 1, +, 2, ;, Eof
    assert_eq!(ks.len(), 5);
    assert!(matches!(ks[0], TokenKind::IntegerLiteral { value: 1, .. }));
    assert!(matches!(ks[1], TokenKind::Plus));
    assert!(matches!(ks[2], TokenKind::IntegerLiteral { value: 2, .. }));
    assert!(matches!(ks[3], TokenKind::Semicolon));
    assert!(matches!(ks[4], TokenKind::Eof));
}

#[test]
fn function_like_nested_parens_in_argument() {
    // Commas inside parentheses do NOT split arguments: `(1, 2)` is
    // a single argument, `3` is the second.
    let (mut pp, out) = run("#define ADD(a, b) a + b\nADD((1, 2), 3);");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: (, 1, ,, 2, ), +, 3, ;, Eof
    assert_eq!(ks.len(), 9);
    assert!(matches!(ks[0], TokenKind::LeftParen));
    assert!(matches!(ks[1], TokenKind::IntegerLiteral { value: 1, .. }));
    assert!(matches!(ks[2], TokenKind::Comma));
    assert!(matches!(ks[3], TokenKind::IntegerLiteral { value: 2, .. }));
    assert!(matches!(ks[4], TokenKind::RightParen));
    assert!(matches!(ks[5], TokenKind::Plus));
    assert!(matches!(ks[6], TokenKind::IntegerLiteral { value: 3, .. }));
    assert!(matches!(ks[7], TokenKind::Semicolon));
    assert!(matches!(ks[8], TokenKind::Eof));
}

#[test]
fn function_like_empty_argument_for_one_param_macro() {
    // F() for a one-param macro is one empty argument — the
    // parameter use vanishes from the output.
    let (mut pp, out) = run("#define F(x) < x >\nF();");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: <, >, ;, Eof
    assert_eq!(ks.len(), 4);
    assert!(matches!(ks[0], TokenKind::Less));
    assert!(matches!(ks[1], TokenKind::Greater));
    assert!(matches!(ks[2], TokenKind::Semicolon));
    assert!(matches!(ks[3], TokenKind::Eof));
}

#[test]
fn function_like_zero_arg_invocation_of_zero_param_macro() {
    // NOW() expands to `12345` with zero arguments.
    let (mut pp, out) = run("#define NOW() 12345\nint x = NOW();");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: int, x, =, 12345, ;, Eof
    assert_eq!(ks.len(), 6);
    assert!(matches!(ks[0], TokenKind::Int));
    assert!(matches!(
        ks[3],
        TokenKind::IntegerLiteral { value: 12345, .. }
    ));
}

#[test]
fn function_like_comma_only_produces_two_empty_args() {
    // PAIR(,) on a two-param macro: both args empty.  Output
    // contains the `+` alone (plus `;` and Eof).
    let (mut pp, out) = run("#define PAIR(a, b) a + b\nPAIR(,);");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: +, ;, Eof
    assert_eq!(ks.len(), 3);
    assert!(matches!(ks[0], TokenKind::Plus));
    assert!(matches!(ks[1], TokenKind::Semicolon));
    assert!(matches!(ks[2], TokenKind::Eof));
}

#[test]
fn function_like_argument_used_twice_substitutes_both_sites() {
    // SQUARE(n + 1) → `n + 1 * n + 1`.  No implicit parenthesisation
    // — that's the C preprocessor's textbook gotcha.
    let (mut pp, out) = run("#define SQUARE(x) x * x\nSQUARE(n + 1);");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: n, +, 1, *, n, +, 1, ;, Eof
    assert_eq!(ks.len(), 9);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "n"));
    assert!(matches!(ks[1], TokenKind::Plus));
    assert!(matches!(ks[2], TokenKind::IntegerLiteral { value: 1, .. }));
    assert!(matches!(ks[3], TokenKind::Star));
    assert!(matches!(ks[4], TokenKind::Identifier(ref s) if s == "n"));
    assert!(matches!(ks[5], TokenKind::Plus));
    assert!(matches!(ks[6], TokenKind::IntegerLiteral { value: 1, .. }));
}

#[test]
fn function_like_arguments_are_pre_expanded() {
    // ADD(NUM, 1) with NUM defined as 42 → `42 + 1`.  The argument
    // NUM must be expanded once before substitution (C17
    // §6.10.3.1/1).
    let (mut pp, out) = run("#define NUM 42\n#define ADD(a, b) a + b\nADD(NUM, 1);");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: 42, +, 1, ;, Eof
    assert_eq!(ks.len(), 5);
    assert!(matches!(ks[0], TokenKind::IntegerLiteral { value: 42, .. }));
    assert!(matches!(ks[1], TokenKind::Plus));
    assert!(matches!(ks[2], TokenKind::IntegerLiteral { value: 1, .. }));
}

// -----------------------------------------------------------------
// Variadic macros — §6.10.3/4
// -----------------------------------------------------------------

#[test]
fn variadic_macro_substitutes_va_args_as_remaining_arguments() {
    // LOG("x=%d", x) → printf("x=%d", x)
    let (mut pp, out) = run("#define LOG(fmt, ...) printf(fmt, __VA_ARGS__)\nLOG(\"x=%d\", x);");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: printf, (, "x=%d", ,, x, ), ;, Eof
    assert_eq!(ks.len(), 8);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "printf"));
    assert!(matches!(ks[1], TokenKind::LeftParen));
    assert!(matches!(ks[2], TokenKind::StringLiteral { ref value, .. } if value == "x=%d"));
    assert!(matches!(ks[3], TokenKind::Comma));
    assert!(matches!(ks[4], TokenKind::Identifier(ref s) if s == "x"));
    assert!(matches!(ks[5], TokenKind::RightParen));
    assert!(matches!(ks[6], TokenKind::Semicolon));
    assert!(matches!(ks[7], TokenKind::Eof));
}

#[test]
fn variadic_macro_preserves_commas_between_variadic_arguments() {
    // LOG("%d %d", 1, 2) → printf("%d %d", 1, 2).  The second and
    // later commas go into __VA_ARGS__ unchanged.
    let (mut pp, out) =
        run("#define LOG(fmt, ...) printf(fmt, __VA_ARGS__)\nLOG(\"%d %d\", 1, 2);");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: printf, (, "%d %d", ,, 1, ,, 2, ), ;, Eof
    assert_eq!(ks.len(), 10);
    assert!(matches!(ks[3], TokenKind::Comma));
    assert!(matches!(ks[4], TokenKind::IntegerLiteral { value: 1, .. }));
    assert!(matches!(ks[5], TokenKind::Comma));
    assert!(matches!(ks[6], TokenKind::IntegerLiteral { value: 2, .. }));
}

#[test]
fn variadic_macro_with_no_extra_arguments_leaves_va_args_empty() {
    // LOG("hi") with only the required `fmt` — __VA_ARGS__ is
    // empty, so the output contains only `printf("hi",)`.
    let (mut pp, out) = run("#define LOG(fmt, ...) printf(fmt, __VA_ARGS__)\nLOG(\"hi\");");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: printf, (, "hi", ,, ), ;, Eof
    assert_eq!(ks.len(), 7);
    assert!(matches!(ks[3], TokenKind::Comma));
    assert!(matches!(ks[4], TokenKind::RightParen));
}

// -----------------------------------------------------------------
// Interaction with surrounding tokens and rescan
// -----------------------------------------------------------------

#[test]
fn function_like_macro_without_parens_passes_through_unchanged() {
    // F;  — the name has no following `(`, so it stays as an
    // identifier.  This is the "macro not invoked" case.
    let (mut pp, out) = run("#define F(x) x + 1\nF;");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: F, ;, Eof
    assert_eq!(ks.len(), 3);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "F"));
    assert!(matches!(ks[1], TokenKind::Semicolon));
}

#[test]
fn nested_macro_calls_expand_outer_then_inner_on_rescan() {
    // OUTER(5) → INNER(5) → 5 + 1.  The INNER invocation comes out
    // of OUTER's replacement and must be rescanned and expanded.
    let (mut pp, out) = run("#define INNER(x) x + 1\n#define OUTER(x) INNER(x)\nOUTER(5);");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: 5, +, 1, ;, Eof
    assert_eq!(ks.len(), 5);
    assert!(matches!(ks[0], TokenKind::IntegerLiteral { value: 5, .. }));
    assert!(matches!(ks[1], TokenKind::Plus));
    assert!(matches!(ks[2], TokenKind::IntegerLiteral { value: 1, .. }));
}

#[test]
fn function_like_self_recursive_invocation_is_blocked_by_hide_set() {
    // `F(x)` expands to `F(x)` textually — but the inner `F` is
    // marked hidden once F expanded, so the second pass sees it as
    // a plain identifier and stops.
    let (mut pp, out) = run("#define F(x) F(x)\nF(1);");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: F, (, 1, ), ;, Eof
    assert_eq!(ks.len(), 6);
    assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "F"));
    assert!(matches!(ks[1], TokenKind::LeftParen));
    assert!(matches!(ks[2], TokenKind::IntegerLiteral { value: 1, .. }));
    assert!(matches!(ks[3], TokenKind::RightParen));
}

#[test]
fn function_like_invocation_preserves_surrounding_tokens() {
    // int y = ADD(1, 2) * 3;
    // ADD expands to `1 + 2` and the trailing `* 3;` survives.
    let (mut pp, out) = run("#define ADD(a, b) a + b\nint y = ADD(1, 2) * 3;");
    assert!(no_errors(&pp.take_diagnostics()));
    let ks = kinds_of(&out);
    // Expected: int, y, =, 1, +, 2, *, 3, ;, Eof
    assert_eq!(ks.len(), 10);
    assert!(matches!(ks[0], TokenKind::Int));
    assert!(matches!(ks[1], TokenKind::Identifier(ref s) if s == "y"));
    assert!(matches!(ks[2], TokenKind::Equal));
    assert!(matches!(ks[3], TokenKind::IntegerLiteral { value: 1, .. }));
    assert!(matches!(ks[4], TokenKind::Plus));
    assert!(matches!(ks[5], TokenKind::IntegerLiteral { value: 2, .. }));
    assert!(matches!(ks[6], TokenKind::Star));
    assert!(matches!(ks[7], TokenKind::IntegerLiteral { value: 3, .. }));
}

#[test]
fn function_like_wrong_arg_count_reports_error() {
    // ADD(1) — too few arguments.  An error diagnostic must fire.
    let (mut pp, _out) = run("#define ADD(a, b) a + b\nADD(1);");
    let diags = pp.take_diagnostics();
    assert!(
        diags.iter().any(|d| matches!(d.severity, Severity::Error)),
        "expected an arity error, got {diags:?}"
    );
}

#[test]
fn function_like_unterminated_arg_list_reports_error() {
    // No closing `)` before EOF — error fires.
    let (mut pp, _out) = run("#define F(x) x\nF(abc");
    let diags = pp.take_diagnostics();
    assert!(
        diags.iter().any(|d| matches!(d.severity, Severity::Error)),
        "expected unterminated-argument-list error, got {diags:?}"
    );
}
