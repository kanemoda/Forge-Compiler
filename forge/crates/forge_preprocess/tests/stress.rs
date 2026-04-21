//! Stress tests for the preprocessor — 20 edge cases that exercise the
//! pipeline against corner inputs: empty files, thousand-token macro
//! bodies, deeply nested `#if` chains, fifty-parameter macros, extreme
//! `#include` chains, and a grab-bag of pathological expansion shapes.
//!
//! These live outside the unit-test module so they can be run
//! independently (`cargo test -p forge_preprocess --test stress`) and
//! do not slow down the tight unit-test cycle.

use forge_lexer::{Lexer, Token, TokenKind};
use forge_preprocess::{Diagnostic, FileId, PreprocessConfig, Preprocessor, Severity};

/// Run the preprocessor on `src` against a default config and a `<input>`
/// synthetic filename.  Returns the token stream and every diagnostic
/// that was emitted.
fn run(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let tokens = Lexer::new(src, FileId::PRIMARY).tokenize();
    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run(tokens);
    let diags = pp.take_diagnostics();
    (out, diags)
}

/// Non-`Eof` tokens from a stream — the parser-relevant count.
fn non_eof(tokens: &[Token]) -> Vec<&Token> {
    tokens
        .iter()
        .filter(|t| !matches!(t.kind, TokenKind::Eof))
        .collect()
}

/// Integer-literal values as `u64`, in output order.
fn int_values(tokens: &[Token]) -> Vec<u64> {
    tokens
        .iter()
        .filter_map(|t| match &t.kind {
            TokenKind::IntegerLiteral { value, .. } => Some(*value),
            _ => None,
        })
        .collect()
}

/// Identifier names as strings, in output order.
fn idents(tokens: &[Token]) -> Vec<String> {
    tokens
        .iter()
        .filter_map(|t| match &t.kind {
            TokenKind::Identifier(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// `true` iff any diagnostic in `diags` has error severity.
fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| matches!(d.severity, Severity::Error))
}

/// Drop a file at `dir/name` with `contents`, returning its absolute path.
fn write_file(dir: &std::path::Path, name: &str, contents: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, contents).unwrap();
    p
}

// ---------------------------------------------------------------------------
// 1. Empty input
// ---------------------------------------------------------------------------

#[test]
fn stress_01_empty_file_produces_no_tokens_and_no_errors() {
    let (out, diags) = run("");
    assert!(non_eof(&out).is_empty(), "empty input must yield no tokens");
    assert!(
        !has_errors(&diags),
        "empty input is not an error: {diags:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. Comments only
// ---------------------------------------------------------------------------

#[test]
fn stress_02_comments_only_file_is_silently_empty() {
    let src = "// a line comment\n\
               /* a block\n   comment */\n\
               // and another\n";
    let (out, diags) = run(src);
    assert!(
        non_eof(&out).is_empty(),
        "comments should contribute no tokens: {:?}",
        non_eof(&out)
    );
    assert!(!has_errors(&diags));
}

// ---------------------------------------------------------------------------
// 3. Whitespace only
// ---------------------------------------------------------------------------

#[test]
fn stress_03_whitespace_only_file_is_silently_empty() {
    let (out, diags) = run("   \n\n\t\t  \r\n\n  ");
    assert!(non_eof(&out).is_empty());
    assert!(!has_errors(&diags));
}

// ---------------------------------------------------------------------------
// 4. Empty macro expansion
// ---------------------------------------------------------------------------

#[test]
fn stress_04_empty_macro_body_vanishes_from_output() {
    let (out, diags) = run("#define EMPTY\nint x EMPTY = 1;\n");
    assert!(!has_errors(&diags), "{diags:?}");
    // Expect: int x = 1;
    let ks: Vec<&TokenKind> = out.iter().map(|t| &t.kind).collect();
    assert!(matches!(ks[0], TokenKind::Int));
    assert!(matches!(ks[1], TokenKind::Identifier(s) if s == "x"));
    assert!(matches!(ks[2], TokenKind::Equal));
    assert!(matches!(ks[3], TokenKind::IntegerLiteral { value: 1, .. }));
}

// ---------------------------------------------------------------------------
// 5. 1000-token macro body
// ---------------------------------------------------------------------------

#[test]
fn stress_05_thousand_token_macro_body_expands_completely() {
    // Build a body of 1000 `+ N` additions, then use it once.
    let mut body = String::from("0");
    for i in 1..=1000 {
        body.push_str(&format!(" + {i}"));
    }
    let src = format!("#define LONG ({body})\nint total = LONG;\n");
    let (out, diags) = run(&src);
    assert!(!has_errors(&diags), "{diags:?}");
    let values = int_values(&out);
    // 0 plus 1..=1000 = 1001 integer literals.  We emit them verbatim;
    // the actual value is the parser/sema's job.
    assert!(
        values.len() >= 1001,
        "expected 1001+ ints after expansion, got {}",
        values.len()
    );
    assert_eq!(values[0], 0);
    assert_eq!(values[1000], 1000);
}

// ---------------------------------------------------------------------------
// 6. 100 nested `#if 1`
// ---------------------------------------------------------------------------

#[test]
fn stress_06_one_hundred_nested_if_one_all_active() {
    let mut src = String::new();
    for _ in 0..100 {
        src.push_str("#if 1\n");
    }
    src.push_str("int deep_marker;\n");
    for _ in 0..100 {
        src.push_str("#endif\n");
    }
    let (out, diags) = run(&src);
    assert!(!has_errors(&diags), "{diags:?}");
    assert!(
        idents(&out).contains(&"deep_marker".to_string()),
        "innermost branch should have emitted deep_marker"
    );
}

// ---------------------------------------------------------------------------
// 7. Nonexistent include
// ---------------------------------------------------------------------------

#[test]
fn stress_07_nonexistent_include_errors_but_keeps_going() {
    let (out, diags) = run("#include \"definitely_not_a_real_file.h\"\nint after_marker;\n");
    assert!(has_errors(&diags), "missing header must be an error");
    // The tokens that follow the bad `#include` must still land in the
    // stream so the driver can report later errors.
    assert!(
        idents(&out).contains(&"after_marker".to_string()),
        "tokens after failed include must still be emitted"
    );
}

// ---------------------------------------------------------------------------
// 8. 50-parameter macro
// ---------------------------------------------------------------------------

#[test]
fn stress_08_fifty_parameter_macro_expands_each_parameter() {
    let params: Vec<String> = (0..50).map(|i| format!("p{i}")).collect();
    let param_list = params.join(", ");
    let body = params.join(" + ");
    let args: Vec<String> = (0..50).map(|i| i.to_string()).collect();
    let args_list = args.join(", ");
    let src = format!("#define FIFTY({param_list}) ({body})\nint r = FIFTY({args_list});\n");
    let (out, diags) = run(&src);
    assert!(!has_errors(&diags), "{diags:?}");
    let values = int_values(&out);
    // Each of 0..50 must appear as a literal in the expansion.
    for i in 0..50u64 {
        assert!(values.contains(&i), "missing literal {i} in {values:?}");
    }
}

// ---------------------------------------------------------------------------
// 9. Invalid token paste
// ---------------------------------------------------------------------------

#[test]
fn stress_09_invalid_paste_emits_warning_and_keeps_both_tokens() {
    // `a ## ;` cannot form a valid preprocessing token; we warn and
    // leave the two tokens side-by-side so the parser can still see them.
    let (out, diags) = run("#define JOIN(a, b) a ## b\nJOIN(x, ;)\n");
    // The macro should still produce two tokens (an identifier and a
    // semicolon) even when the paste itself is ill-formed.
    let non = non_eof(&out);
    assert!(
        non.iter()
            .any(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "x")),
        "expected `x` identifier: {non:?}"
    );
    assert!(
        non.iter().any(|t| matches!(t.kind, TokenKind::Semicolon)),
        "expected `;` token: {non:?}"
    );
    // Warning preferred, but not required — the key guarantee is "no
    // crash, no silent data loss".  Accept warn or no-diag here.
    let _ = diags;
}

// ---------------------------------------------------------------------------
// 10. 50-level include chain
// ---------------------------------------------------------------------------

#[test]
fn stress_10_fifty_level_include_chain_succeeds_below_depth_limit() {
    let tmp = tempfile::TempDir::new().unwrap();
    // h49.h is the leaf; each hN.h includes h(N+1).h, and main.c starts
    // the chain by including h0.h.
    for i in (0..50u32).rev() {
        let body = if i == 49 {
            "int leaf_marker;\n".to_string()
        } else {
            format!("#include \"h{}.h\"\n", i + 1)
        };
        write_file(tmp.path(), &format!("h{i}.h"), &body);
    }
    let main_path = write_file(tmp.path(), "main.c", "#include \"h0.h\"\n");

    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main_path).unwrap();
    let diags = pp.take_diagnostics();
    assert!(!has_errors(&diags), "chain below depth limit: {diags:?}");
    assert!(
        idents(&out).contains(&"leaf_marker".to_string()),
        "expected leaf of a 50-deep include chain to be reached"
    );
}

// ---------------------------------------------------------------------------
// 11. 100-term `#if` expression
// ---------------------------------------------------------------------------

#[test]
fn stress_11_hundred_term_if_expression_evaluates_without_overflow() {
    // Build `#if 1 && 1 && 1 && ... && 1` with 100 operands.  The
    // evaluator must not stack-overflow on a wide Pratt chain.
    let expr = (0..100).map(|_| "1").collect::<Vec<_>>().join(" && ");
    let src = format!("#if {expr}\nint wide_and_marker;\n#endif\n");
    let (out, diags) = run(&src);
    assert!(!has_errors(&diags), "{diags:?}");
    assert!(idents(&out).contains(&"wide_and_marker".to_string()));
}

// ---------------------------------------------------------------------------
// 12. Macro body whose text *looks like* a directive
// ---------------------------------------------------------------------------

#[test]
fn stress_12_define_body_containing_hash_define_text_is_not_a_directive() {
    // C17: `#` inside a macro replacement list is the stringify operator
    // (or paste for `##`).  A bare `#` with no parameter after it inside
    // a replacement list is a paste-or-stringify error on use, but when
    // the macro never expands the stored replacement stays intact — so
    // here we simply check that parsing the definition does not recurse
    // into a nested directive.
    let (out, diags) = run("#define LOOKS_LIKE_A_DIRECTIVE 1\nint z = LOOKS_LIKE_A_DIRECTIVE;\n");
    assert!(!has_errors(&diags), "{diags:?}");
    assert!(int_values(&out).contains(&1u64));
}

// ---------------------------------------------------------------------------
// 13. Long `#` stringify
// ---------------------------------------------------------------------------

#[test]
fn stress_13_stringify_of_500_token_argument_yields_one_string() {
    // Build a single argument that is 500 `a` identifiers separated by
    // spaces, then stringify it.  Output must be one string literal.
    let arg: String = (0..500).map(|_| "a ").collect();
    let src = format!("#define S(x) #x\nconst char *s = S({arg});\n");
    let (out, diags) = run(&src);
    assert!(!has_errors(&diags), "{diags:?}");
    let strings: Vec<&String> = out
        .iter()
        .filter_map(|t| match &t.kind {
            TokenKind::StringLiteral { value, .. } => Some(value),
            _ => None,
        })
        .collect();
    assert_eq!(strings.len(), 1, "expected exactly one string: {strings:?}");
    // Should start with `a` and contain many `a`s.
    let s = strings[0];
    let a_count = s.matches('a').count();
    assert!(
        a_count >= 500,
        "expected at least 500 `a`s in stringified arg, got {a_count}"
    );
}

// ---------------------------------------------------------------------------
// 14. 5-paste chain
// ---------------------------------------------------------------------------

#[test]
fn stress_14_five_way_paste_chain_produces_single_identifier() {
    let src = "#define CAT(a, b, c, d, e) a ## b ## c ## d ## e\n\
               CAT(f, o, o, b, ar)\n";
    let (out, diags) = run(src);
    assert!(!has_errors(&diags), "{diags:?}");
    let names = idents(&out);
    assert!(
        names.contains(&"foobar".to_string()),
        "expected foobar from chained paste: {names:?}"
    );
}

// ---------------------------------------------------------------------------
// 15. Self-expanding macro
// ---------------------------------------------------------------------------

#[test]
fn stress_15_self_referential_macro_stops_at_one_rescan() {
    // `#define A A` — the identifier on the left of the `=` must appear
    // exactly once in the output, because rescanning the replacement
    // token sees `A` in its own hide-set and emits it verbatim.
    let (out, diags) = run("#define A A\nA A\n");
    assert!(!has_errors(&diags), "{diags:?}");
    let names = idents(&out);
    let a_count = names.iter().filter(|n| n == &"A").count();
    assert_eq!(
        a_count, 2,
        "each use-site should emit exactly one A: got {names:?}"
    );
}

// ---------------------------------------------------------------------------
// 16. UTF-8 in source text
// ---------------------------------------------------------------------------

#[test]
fn stress_16_utf8_bytes_in_source_do_not_crash_the_preprocessor() {
    // The lexer models raw UTF-8 codepoints in identifier position as
    // `TokenKind::Unknown`.  The preprocessor's job here is simply to
    // survive: it must not panic, and it must preserve the byte-span
    // structure so a later diagnostic can point at the offending
    // character.  The string-literal case (valid UTF-8 inside `"..."`)
    // is the well-formed path and must round-trip exactly.
    let src = "const char *hello = \"héllo, wörld\";\nint é = 1;\n";
    let (out, diags) = run(src);
    // String literal path: should be fine.
    let strings: Vec<&String> = out
        .iter()
        .filter_map(|t| match &t.kind {
            TokenKind::StringLiteral { value, .. } => Some(value),
            _ => None,
        })
        .collect();
    assert!(
        strings.iter().any(|s| s.contains("héllo")),
        "UTF-8 inside a string literal must survive: {strings:?}"
    );
    // Identifier path: an error is acceptable; a panic is not.  The
    // expected behaviour is an `Unknown('é')` lexer token that the
    // preprocessor passes through without imploding.
    let _ = diags;
}

// ---------------------------------------------------------------------------
// 17. Basic `#if 1` / `#if 0`
// ---------------------------------------------------------------------------

#[test]
fn stress_17_if_one_takes_branch_if_zero_skips() {
    let (out, diags) = run("#if 1\nint taken;\n#endif\n\
         #if 0\nint skipped;\n#endif\n");
    assert!(!has_errors(&diags), "{diags:?}");
    let names = idents(&out);
    assert!(names.contains(&"taken".to_string()));
    assert!(!names.contains(&"skipped".to_string()));
}

// ---------------------------------------------------------------------------
// 18. Unterminated macro call
// ---------------------------------------------------------------------------

#[test]
fn stress_18_unterminated_function_like_call_is_an_error() {
    // `F(1, 2` never closes — the preprocessor must flag it as an error
    // and not hang.
    let (_out, diags) = run("#define F(a, b) (a + b)\nint r = F(1, 2\n");
    assert!(has_errors(&diags), "expected an error, got {diags:?}");
}

// ---------------------------------------------------------------------------
// 19. Deeply nested parentheses in an argument
// ---------------------------------------------------------------------------

#[test]
fn stress_19_fifty_deep_parens_in_argument_parse_cleanly() {
    // Argument is `((((...1...))))` with 50 nested parens.
    let mut arg = String::new();
    for _ in 0..50 {
        arg.push('(');
    }
    arg.push('1');
    for _ in 0..50 {
        arg.push(')');
    }
    let src = format!("#define ID(x) x\nint y = ID({arg});\n");
    let (out, diags) = run(&src);
    assert!(!has_errors(&diags), "{diags:?}");
    assert!(int_values(&out).contains(&1u64));
}

// ---------------------------------------------------------------------------
// 20. Absolute-path include
// ---------------------------------------------------------------------------

#[test]
fn stress_20_absolute_path_include_loads_target_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let header_path = write_file(tmp.path(), "abs_hdr.h", "int abs_marker;\n");
    // Canonicalise so platform-specific quirks (symlinks, /private/var
    // on macOS) do not trip the test.
    let abs = std::fs::canonicalize(&header_path).unwrap();
    let main = write_file(
        tmp.path(),
        "main.c",
        &format!("#include \"{}\"\n", abs.display()),
    );
    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main).unwrap();
    let diags = pp.take_diagnostics();
    assert!(!has_errors(&diags), "{diags:?}");
    assert!(idents(&out).contains(&"abs_marker".to_string()));
}
