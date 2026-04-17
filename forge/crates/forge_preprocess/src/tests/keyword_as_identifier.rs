//! Preprocessor treats C keywords as ordinary identifiers.
//!
//! C17 §6.10 is specified over preprocessing tokens, where reserved
//! words do not yet exist: the preprocessor sees identifiers and must
//! accept keyword spellings (`_Noreturn`, `int`, `_Static_assert`, …)
//! anywhere an identifier is allowed.  The lexer has already classified
//! those spellings into their final [`forge_lexer::TokenKind`] variants
//! by the time the preprocessor runs, so every identifier-handling path
//! inside the preprocessor must consult
//! [`forge_lexer::TokenKind::identifier_spelling`] rather than matching
//! on [`forge_lexer::TokenKind::Identifier`] alone.  These tests pin
//! that contract in place for the behaviours the rewrite fixed.

use forge_lexer::TokenKind;

use super::helpers::*;

// ---------------------------------------------------------------------------
// #define / #undef
// ---------------------------------------------------------------------------

#[test]
fn define_accepts_a_keyword_spelling_as_the_macro_name() {
    // `_Noreturn` is a C11 keyword but the preprocessor is free to
    // shadow it as a macro — glibc's `<stdnoreturn.h>` does exactly
    // this.  The redefinition must expand at the call site.  We use an
    // ordinary placeholder identifier in the body (rather than a real
    // GCC attribute) so the result is not collapsed by the predefined
    // `__attribute__(x)` erasure macro.
    let (mut pp, out) = run("#define _Noreturn NORETURN_MARKER\n_Noreturn\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert!(identifier_names(&out).contains(&"NORETURN_MARKER".to_string()));
}

#[test]
fn undef_accepts_a_keyword_spelling_as_the_macro_name() {
    // After `#undef _Noreturn`, `#ifdef _Noreturn` must be false.  No
    // diagnostic should fire from the `#undef` itself.
    let src = "#define _Noreturn 1\n\
               #undef _Noreturn\n\
               #ifdef _Noreturn\nYES\n#else\nNO\n#endif\n";
    let (mut pp, out) = run(src);
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["NO"]);
}

// ---------------------------------------------------------------------------
// #ifdef / #ifndef / defined()
// ---------------------------------------------------------------------------

#[test]
fn ifdef_matches_a_keyword_spelled_macro_name() {
    let (mut pp, out) = run("#define _Noreturn\n#ifdef _Noreturn\nYES\n#endif\n");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn defined_with_parens_sees_a_keyword_spelled_macro() {
    let src = "#define _Static_assert\n#if defined(_Static_assert)\nYES\n#endif\n";
    let (mut pp, out) = run(src);
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

#[test]
fn defined_without_parens_sees_a_keyword_spelled_macro() {
    let src = "#define _Static_assert\n#if defined _Static_assert\nYES\n#endif\n";
    let (mut pp, out) = run(src);
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["YES"]);
}

// ---------------------------------------------------------------------------
// #if — unknown identifier rule
// ---------------------------------------------------------------------------

#[test]
fn if_treats_an_undefined_keyword_like_an_undefined_identifier() {
    // C17 §6.10.1/4: "identifiers that are not macros … are replaced
    // with the pp-number 0".  Keywords that are not also macros must
    // take that path — they MUST NOT be treated as parse errors.
    let src = "#if _Noreturn\nYES\n#else\nNO\n#endif\n";
    let (mut pp, out) = run(src);
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(identifier_names(&out), vec!["NO"]);
}

// ---------------------------------------------------------------------------
// Macro expansion — keyword as the macro invocation
// ---------------------------------------------------------------------------

#[test]
fn a_keyword_spelled_macro_still_expands_when_used() {
    // Defining `int` as an object-like macro that expands to another
    // token sequence should take effect at every subsequent
    // occurrence of the spelling `int`.
    let (mut pp, out) = run("#define int long\nint\n");
    assert!(no_errors(&pp.take_diagnostics()));
    // The output is a single `long` keyword token.
    let kinds = kinds_of(&non_eof(&out).into_iter().cloned().collect::<Vec<_>>());
    assert_eq!(kinds, vec![TokenKind::Long]);
}
