//! Tests covering the `#` (stringify) operator and `spelling_of`.

use forge_lexer::{IntSuffix, TokenKind};

use super::helpers::*;
use crate::expand::{spelling_of, stringify};
use crate::pp_token::PPToken;

fn wrap(src: &str) -> Vec<PPToken> {
    forge_lexer::lex_fragment(src)
        .into_iter()
        .map(PPToken::new)
        .collect()
}

// -----------------------------------------------------------------
// spelling_of
// -----------------------------------------------------------------

#[test]
fn spelling_of_keywords_and_punctuators_is_canonical() {
    assert_eq!(spelling_of(&TokenKind::Int), "int");
    assert_eq!(spelling_of(&TokenKind::Return), "return");
    assert_eq!(spelling_of(&TokenKind::LeftBrace), "{");
    assert_eq!(spelling_of(&TokenKind::HashHash), "##");
    assert_eq!(spelling_of(&TokenKind::Ellipsis), "...");
}

#[test]
fn spelling_of_identifier_is_its_name() {
    assert_eq!(
        spelling_of(&TokenKind::Identifier("foo_bar".into())),
        "foo_bar"
    );
}

#[test]
fn spelling_of_integer_literal_reattaches_suffix() {
    assert_eq!(
        spelling_of(&TokenKind::IntegerLiteral {
            value: 42,
            suffix: IntSuffix::None
        }),
        "42"
    );
    assert_eq!(
        spelling_of(&TokenKind::IntegerLiteral {
            value: 1,
            suffix: IntSuffix::ULL
        }),
        "1ull"
    );
}

// -----------------------------------------------------------------
// stringify helper (operates directly on PPToken lists)
// -----------------------------------------------------------------

#[test]
fn stringify_identifier_is_just_its_spelling() {
    let arg = wrap("hello");
    assert_eq!(stringify(&arg), "hello");
}

#[test]
fn stringify_preserves_internal_spaces_only_where_has_leading_space_is_set() {
    // "a + b" — the lexer marks `+` and `b` as having leading space.
    let arg = wrap("a + b");
    assert_eq!(stringify(&arg), "a + b");
}

#[test]
fn stringify_concatenates_adjacent_tokens_without_spaces() {
    // `a+b` — no spaces between tokens.
    let arg = wrap("a+b");
    assert_eq!(stringify(&arg), "a+b");
}

#[test]
fn stringify_escapes_inner_quotes_in_string_literal_arg() {
    // The argument is a single StringLiteral token whose spelling is
    // `"hello"`.  Stringification must escape both double quotes.
    let arg = wrap("\"hello\"");
    assert_eq!(stringify(&arg), "\\\"hello\\\"");
}

#[test]
fn stringify_escapes_inner_backslash_in_string_literal_arg() {
    // Source is `"a\nb"` — the lexer decodes `\n` to a newline, but
    // stringify re-emits the source form as `\n`, then escapes the
    // backslash to `\\n`.
    let arg = wrap("\"a\\nb\"");
    assert_eq!(stringify(&arg), "\\\"a\\\\nb\\\"");
}

#[test]
fn stringify_escapes_backslash_in_char_literal_arg() {
    // Source is `'\\'` — the char literal value is a single backslash.
    // Spelling reconstructs `'\\'`; stringify doubles each backslash:
    // `'\\\\'`.
    let arg = wrap("'\\\\'");
    assert_eq!(stringify(&arg), "'\\\\\\\\'");
}

#[test]
fn stringify_does_not_escape_single_quote() {
    let arg = wrap("'a'");
    assert_eq!(stringify(&arg), "'a'");
}

// -----------------------------------------------------------------
// The `#` operator through the preprocessor's full pipeline
// -----------------------------------------------------------------

#[test]
fn stringify_basic_identifier_argument() {
    // STR(hello) → "hello"
    let (mut pp, out) = run("#define STR(x) #x\nSTR(hello)");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(only_string(&out), "hello");
}

#[test]
fn stringify_collapses_whitespace_between_tokens_to_single_spaces() {
    // STR( 1 + 2 ) → "1 + 2"  — leading/trailing whitespace is
    // stripped; interior runs collapse to single spaces.
    let (mut pp, out) = run("#define STR(x) #x\nSTR( 1   +   2 )");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(only_string(&out), "1 + 2");
}

#[test]
fn stringify_uses_raw_argument_not_expansion() {
    // NUM expands to 5, but inside `#x` the argument stays literal:
    // STR(NUM) → "NUM".  C17 §6.10.3.2: `#` uses the *raw* argument
    // tokens — no pre-expansion.
    let (mut pp, out) = run("#define NUM 5\n#define STR(x) #x\nSTR(NUM)");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(only_string(&out), "NUM");
}

#[test]
fn stringify_escapes_embedded_double_quotes() {
    // STR("hello") — the argument's spelling is `"hello"` (7 chars,
    // including the quotes).  Stringify escapes each `"` to `\"`,
    // so the resulting StringLiteral value holds the 9 chars
    // `\"hello\"`.
    let (mut pp, out) = run("#define STR(x) #x\nSTR(\"hello\")");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(only_string(&out), "\\\"hello\\\"");
}

#[test]
fn stringify_escapes_embedded_backslashes() {
    // Source `STR("a\\b")`: the argument is a string literal whose
    // spelling reconstructs as `"a\\b"` (6 chars).  Stringify
    // escapes `"` → `\"` and each `\` → `\\`, giving the 10 chars
    // `\"a\\\\b\"`.
    let (mut pp, out) = run("#define STR(x) #x\nSTR(\"a\\\\b\")");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(only_string(&out), "\\\"a\\\\\\\\b\\\"");
}

#[test]
fn stringify_char_literal_escapes_inner_quotes() {
    // STR('a') → "'a'".  A simple char literal comes out
    // unescaped because single quotes need no protection inside a
    // double-quoted string.
    let (mut pp, out) = run("#define STR(x) #x\nSTR('a')");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(only_string(&out), "'a'");
}

#[test]
fn stringify_empty_argument_produces_empty_string() {
    // STR() → ""
    let (mut pp, out) = run("#define STR(x) #x\nSTR()");
    assert!(no_errors(&pp.take_diagnostics()));
    assert_eq!(only_string(&out), "");
}
