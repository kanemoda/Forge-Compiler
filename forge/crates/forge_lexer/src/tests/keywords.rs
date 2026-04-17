//! Keyword and identifier recognition.

use super::helpers::*;
use crate::TokenKind;

#[test]
fn bare_identifier() {
    let toks = kinds("foo");
    assert_eq!(toks, vec![TokenKind::Identifier("foo".to_string())]);
}

#[test]
fn identifier_with_digits_and_underscores() {
    let toks = kinds("_foo_bar123");
    assert_eq!(toks, vec![TokenKind::Identifier("_foo_bar123".to_string())]);
}

#[test]
fn identifier_does_not_include_trailing_punctuation() {
    let toks = kinds("foo;");
    assert_eq!(
        toks,
        vec![
            TokenKind::Identifier("foo".to_string()),
            TokenKind::Semicolon,
        ]
    );
}

#[test]
fn keyword_prefix_is_still_an_identifier() {
    // `integer` should not be `int` + `eger`.
    let toks = kinds("integer");
    assert_eq!(toks, vec![TokenKind::Identifier("integer".to_string())]);
}

#[test]
fn all_c17_keywords() {
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
    for (src, expected) in cases {
        let got = kinds(src);
        assert_eq!(got, vec![expected.clone()], "keyword `{src}` mis-lexed");
    }
}

#[test]
fn keyword_is_case_sensitive() {
    // `Int`, `INT` are identifiers, not the `int` keyword.
    assert_eq!(kinds("Int"), vec![TokenKind::Identifier("Int".to_string())]);
    assert_eq!(kinds("INT"), vec![TokenKind::Identifier("INT".to_string())]);
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
        let k = kinds(src);
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

#[test]
fn identifier_spelling_round_trips_every_keyword() {
    // Every keyword's spelling should round-trip through lookup_keyword,
    // and every keyword TokenKind should report its canonical spelling.
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
    for (spelling, kind) in cases {
        assert_eq!(
            kind.identifier_spelling(),
            Some(*spelling),
            "{spelling}: identifier_spelling mismatch",
        );
        assert!(kind.is_identifier_like(), "{spelling}: is_identifier_like");
    }
}

#[test]
fn identifier_spelling_yields_the_spelling_for_plain_identifiers() {
    let ident = TokenKind::Identifier("foo_123".to_string());
    assert_eq!(ident.identifier_spelling(), Some("foo_123"));
    assert!(ident.is_identifier_like());
}

#[test]
fn identifier_spelling_is_none_for_non_identifier_tokens() {
    for kind in [
        TokenKind::Plus,
        TokenKind::LeftParen,
        TokenKind::Eof,
        TokenKind::IntegerLiteral {
            value: 0,
            suffix: crate::IntSuffix::None,
        },
        TokenKind::StringLiteral {
            value: "s".to_string(),
            prefix: crate::StringPrefix::None,
        },
    ] {
        assert_eq!(kind.identifier_spelling(), None);
        assert!(!kind.is_identifier_like());
    }
}
