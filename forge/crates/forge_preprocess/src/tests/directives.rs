//! Main-loop directive dispatch plus `#error`, `#warning`, `#line`, `#pragma`, and `_Pragma`.

use forge_diagnostics::Severity;
use forge_lexer::TokenKind;
use tempfile::TempDir;

use super::helpers::*;
use crate::{preprocess, PreprocessConfig, Preprocessor};

// -----------------------------------------------------------------
// Main loop pass-through
// -----------------------------------------------------------------

#[test]
fn non_directive_tokens_pass_through_in_order() {
    let src = "int x = 42;";
    let (mut pp, out) = run(src);
    assert!(pp.take_diagnostics().is_empty());
    // out = int, x, =, 42, ;, Eof
    let kinds: Vec<_> = out.iter().map(|t| t.kind.clone()).collect();
    assert!(matches!(kinds[0], TokenKind::Int));
    assert!(matches!(kinds[1], TokenKind::Identifier(ref s) if s == "x"));
    assert!(matches!(kinds[2], TokenKind::Equal));
    assert!(matches!(
        kinds[3],
        TokenKind::IntegerLiteral { value: 42, .. }
    ));
    assert!(matches!(kinds[4], TokenKind::Semicolon));
    assert!(matches!(kinds[5], TokenKind::Eof));
}

#[test]
fn directive_lines_are_removed_from_the_output() {
    // The `#define` line must not appear in the output stream.
    let (_, out) = run("#define FOO 42\nint x;");
    let kinds: Vec<_> = out.iter().map(|t| t.kind.clone()).collect();
    // Expected: int, x, ;, Eof  — no Hash, no `define`, no `FOO`.
    assert_eq!(kinds.len(), 4);
    assert!(matches!(kinds[0], TokenKind::Int));
    assert!(matches!(kinds[1], TokenKind::Identifier(ref s) if s == "x"));
    assert!(matches!(kinds[2], TokenKind::Semicolon));
    assert!(matches!(kinds[3], TokenKind::Eof));
}

#[test]
fn hash_not_at_start_of_line_passes_through_as_a_regular_token() {
    // Here the `#` appears mid-line, so the preprocessor must NOT
    // treat it as a directive; it passes through as a Hash token.
    let (mut pp, out) = run("int a ; # b\n");
    assert!(
        pp.take_diagnostics().is_empty(),
        "unexpected diags: {:?}",
        pp.take_diagnostics()
    );
    let kinds: Vec<_> = out.iter().map(|t| t.kind.clone()).collect();
    assert!(matches!(kinds[0], TokenKind::Int));
    assert!(matches!(kinds[1], TokenKind::Identifier(ref s) if s == "a"));
    assert!(matches!(kinds[2], TokenKind::Semicolon));
    // The `#` retains its token kind.
    assert!(matches!(kinds[3], TokenKind::Hash));
    // And it is NOT at start of line.
    assert!(!out[3].at_start_of_line);
    assert!(matches!(kinds[4], TokenKind::Identifier(ref s) if s == "b"));
    assert!(matches!(kinds[5], TokenKind::Eof));
}

#[test]
fn hash_hash_token_passes_through_regardless_of_position() {
    let (_, out) = run("## x\n");
    let kinds: Vec<_> = out.iter().map(|t| t.kind.clone()).collect();
    // `##` is HashHash, not Hash, so it is never treated as a
    // directive.
    assert!(matches!(kinds[0], TokenKind::HashHash));
    assert!(matches!(kinds[1], TokenKind::Identifier(ref s) if s == "x"));
    assert!(matches!(kinds[2], TokenKind::Eof));
}

#[test]
fn null_directive_is_valid_and_produces_no_diagnostic() {
    // A `#` alone on a line — valid C17 null directive.
    let (mut pp, out) = run("#\nint x;");
    assert!(
        pp.take_diagnostics().is_empty(),
        "null directive must not produce diagnostics"
    );
    let kinds: Vec<_> = out.iter().map(|t| t.kind.clone()).collect();
    assert!(matches!(kinds[0], TokenKind::Int));
    assert!(matches!(kinds[1], TokenKind::Identifier(ref s) if s == "x"));
}

#[test]
fn null_directive_followed_by_real_code_passes_code_through() {
    let (mut pp, out) = run("#\nint x = 1;\n");
    assert!(
        pp.take_diagnostics().is_empty(),
        "null directive must not produce diagnostics"
    );
    let kinds: Vec<_> = out
        .iter()
        .map(|t| t.kind.clone())
        .filter(|k| !matches!(k, TokenKind::Eof))
        .collect();
    assert!(matches!(kinds[0], TokenKind::Int));
    assert!(matches!(kinds[1], TokenKind::Identifier(ref s) if s == "x"));
    assert!(matches!(kinds[2], TokenKind::Equal));
    assert!(matches!(
        kinds[3],
        TokenKind::IntegerLiteral { value: 1, .. }
    ));
    assert!(matches!(kinds[4], TokenKind::Semicolon));
}

#[test]
fn three_consecutive_null_directives_are_no_ops() {
    let (mut pp, out) = run("#\n#\n#\nint x;\n");
    assert!(
        pp.take_diagnostics().is_empty(),
        "consecutive null directives must not produce diagnostics"
    );
    let kinds: Vec<_> = out
        .iter()
        .map(|t| t.kind.clone())
        .filter(|k| !matches!(k, TokenKind::Eof))
        .collect();
    assert_eq!(kinds.len(), 3);
    assert!(matches!(kinds[0], TokenKind::Int));
    assert!(matches!(kinds[1], TokenKind::Identifier(ref s) if s == "x"));
    assert!(matches!(kinds[2], TokenKind::Semicolon));
}

#[test]
fn unknown_directive_produces_an_error_diagnostic() {
    let (mut pp, _) = run("#frobnicate foo\n");
    let diags = pp.take_diagnostics();
    assert_eq!(diags.len(), 1);
    assert!(matches!(diags[0].severity, Severity::Error));
    assert!(diags[0].message.contains("frobnicate"));
}

#[test]
fn error_directive_emits_an_error_and_marks_has_errors() {
    // `#error` is now wired up: it emits an Error and sets the
    // preprocessor's `has_errors` flag.  Processing continues past
    // the directive.
    let (mut pp, _) = run("#error oh no\nint x;\n");
    let diags = pp.take_diagnostics();
    assert!(
        diags
            .iter()
            .any(|d| matches!(d.severity, Severity::Error) && d.message.contains("oh no")),
        "expected an #error diagnostic mentioning `oh no`: {diags:?}"
    );
    assert!(pp.has_errors());
}

// -----------------------------------------------------------------
// preprocess() entry point
// -----------------------------------------------------------------

#[test]
fn preprocess_returns_ok_when_no_errors_emitted() {
    let out = preprocess(lex("int x;"), PreprocessConfig::default());
    assert!(out.is_ok());
}

#[test]
fn preprocess_returns_err_containing_diagnostics_on_error() {
    let out = preprocess(lex("#frobnicate\n"), PreprocessConfig::default());
    let diags = out.expect_err("expected errors");
    assert!(diags.iter().any(|d| matches!(d.severity, Severity::Error)));
}

#[test]
fn preprocess_returns_ok_when_only_warnings_are_emitted() {
    let out = preprocess(
        lex("#define X 1\n#define X 2\n"),
        PreprocessConfig::default(),
    );
    // Redefinition only produces a warning, so preprocess() must
    // still return Ok(...).
    assert!(out.is_ok());
}

// ---------- B. #error ----------

#[test]
fn error_directive_emits_error_with_message_from_body() {
    let (mut pp, _) = run("#error this is broken\n");
    let diags = pp.take_diagnostics();
    let errs = errors_of(&diags);
    assert_eq!(errs.len(), 1, "expected exactly one error: {diags:?}");
    assert!(
        errs[0].message.contains("this is broken"),
        "unexpected message: {:?}",
        errs[0].message
    );
    assert!(pp.has_errors());
}

#[test]
fn error_directive_is_scoped_to_active_conditionals() {
    // Inside `#if 0` the body is not parsed as a directive at all —
    // so `#error` in a skipped branch must never fire.
    let (mut pp, _) = run("#if 0\n#error nope\n#endif\nint x;\n");
    let diags = pp.take_diagnostics();
    assert!(
        errors_of(&diags).is_empty(),
        "expected no errors in skipped branch: {diags:?}"
    );
    assert!(!pp.has_errors());
}

#[test]
fn error_directive_does_not_stop_subsequent_processing() {
    // After the `#error`, the translation unit must keep parsing so
    // later issues are still reported and later tokens still make it
    // into the output.
    let (pp, out) = run("#error first\nint keep_going;\n");
    assert!(pp.has_errors());
    let names = identifier_names(&out);
    assert!(
        names.contains(&"keep_going".to_string()),
        "tokens after #error must still be emitted: {names:?}"
    );
}

#[test]
fn error_directive_does_not_macro_expand_its_argument() {
    // C17 §6.10.5: the tokens on the `#error` line are used
    // verbatim.  Even if a macro in scope would otherwise expand,
    // the message must mention the *macro name*, not its body.
    let (mut pp, _) = run("#define FOO 42\n#error FOO is bad\n");
    let diags = pp.take_diagnostics();
    let errs = errors_of(&diags);
    assert_eq!(errs.len(), 1);
    assert!(
        errs[0].message.contains("FOO"),
        "macro name should survive: {:?}",
        errs[0].message
    );
    assert!(
        !errs[0].message.contains("42"),
        "macro body must not appear: {:?}",
        errs[0].message
    );
}

#[test]
fn error_directive_span_points_at_the_hash_token() {
    // The source `#error msg\n` begins at byte 0; the `#` occupies
    // bytes 0..1.
    let (mut pp, _) = run("#error oops\n");
    let diags = pp.take_diagnostics();
    let errs = errors_of(&diags);
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].span.start, 0, "span should be the `#` token");
    assert_eq!(errs[0].span.end, 1, "span should be the `#` token");
}

#[test]
fn empty_error_directive_still_reports_an_error() {
    let (mut pp, _) = run("#error\n");
    let diags = pp.take_diagnostics();
    let errs = errors_of(&diags);
    assert_eq!(errs.len(), 1);
    assert!(pp.has_errors());
}

#[test]
fn preprocess_function_propagates_error_directive_as_err() {
    let tokens = lex("#error bad\n");
    let result = preprocess(tokens, PreprocessConfig::default());
    assert!(result.is_err(), "#error must make preprocess return Err");
}

// ---------- C. #warning ----------

#[test]
fn warning_directive_emits_warning_not_error() {
    let (mut pp, _) = run("#warning be careful\n");
    let diags = pp.take_diagnostics();
    let warns = warnings_of(&diags);
    assert_eq!(warns.len(), 1, "expected exactly one warning: {diags:?}");
    assert!(warns[0].message.contains("be careful"));
    assert!(
        errors_of(&diags).is_empty(),
        "must not produce an error: {diags:?}"
    );
    assert!(!pp.has_errors(), "has_errors must stay false");
}

#[test]
fn warning_directive_is_scoped_to_active_conditionals() {
    let (mut pp, _) = run("#if 0\n#warning nope\n#endif\n");
    let diags = pp.take_diagnostics();
    assert!(warnings_of(&diags).is_empty());
}

#[test]
fn warning_directive_does_not_macro_expand_its_argument() {
    let (mut pp, _) = run("#define FOO 42\n#warning FOO detected\n");
    let diags = pp.take_diagnostics();
    let warns = warnings_of(&diags);
    assert_eq!(warns.len(), 1);
    assert!(warns[0].message.contains("FOO"));
    assert!(!warns[0].message.contains("42"));
}

#[test]
fn preprocess_function_accepts_warning_directive() {
    // `#warning` must not cause `preprocess` to return `Err`.
    let tokens = lex("#warning careful\nint x;\n");
    let result = preprocess(tokens, PreprocessConfig::default());
    assert!(result.is_ok(), "#warning alone should be Ok: {result:?}");
}

// ---------- D. #line ----------

#[test]
fn line_directive_sets_line_for_next_line() {
    // After `#line 100`, the very next source line reports as 100.
    let n = line_value_from_macro("#line 100\n__LINE__\n");
    assert_eq!(n, 100);
}

#[test]
fn line_directive_advances_by_physical_lines_after_anchor() {
    // `#line 100` on physical line 1 → physical line 2 is reported
    // as 100; physical line 3 is 101; and so on.
    let n = line_value_from_macro("#line 100\n\n__LINE__\n");
    assert_eq!(n, 101);
}

#[test]
fn line_directive_sets_filename_for_file_macro() {
    let (_, out) = run("#line 1 \"virtual.c\"\n__FILE__\n");
    // First non-Eof token is a StringLiteral with the new filename.
    let tok = out
        .iter()
        .find(|t| matches!(t.kind, TokenKind::StringLiteral { .. }))
        .expect("expected a string literal in output");
    match &tok.kind {
        TokenKind::StringLiteral { value, .. } => assert_eq!(value, "virtual.c"),
        _ => unreachable!(),
    }
}

#[test]
fn line_directive_zero_is_rejected() {
    let (mut pp, _) = run("#line 0\n");
    let diags = pp.take_diagnostics();
    let errs = errors_of(&diags);
    assert_eq!(errs.len(), 1);
    assert!(
        errs[0].message.contains("invalid line number"),
        "unexpected message: {:?}",
        errs[0].message
    );
}

#[test]
fn line_directive_exceeding_max_is_rejected() {
    let (mut pp, _) = run("#line 2147483648\n");
    let diags = pp.take_diagnostics();
    assert_eq!(errors_of(&diags).len(), 1);
}

#[test]
fn line_directive_non_integer_is_rejected() {
    let (mut pp, _) = run("#line oops\n");
    let diags = pp.take_diagnostics();
    let errs = errors_of(&diags);
    assert_eq!(errs.len(), 1);
    assert!(
        errs[0].message.contains("integer"),
        "expected integer-related message: {:?}",
        errs[0].message
    );
}

#[test]
fn line_directive_empty_body_is_rejected() {
    let (mut pp, _) = run("#line\n");
    let diags = pp.take_diagnostics();
    assert_eq!(errors_of(&diags).len(), 1);
}

#[test]
fn line_directive_macro_expands_its_arguments() {
    // `#line L F` where `L` expands to `50` and `F` to `"gen.c"`.
    let (_, out) = run("#define L 50\n#define F \"gen.c\"\n#line L F\n__LINE__ __FILE__\n");
    let mut seen_line = false;
    let mut seen_file = false;
    for t in out {
        if let TokenKind::IntegerLiteral { value, .. } = t.kind {
            assert_eq!(value, 50);
            seen_line = true;
        }
        if let TokenKind::StringLiteral { value, .. } = &t.kind {
            if value == "gen.c" {
                seen_file = true;
            }
        }
    }
    assert!(seen_line, "__LINE__ should report 50");
    assert!(seen_file, "__FILE__ should report gen.c");
}

#[test]
fn line_directive_does_not_leak_out_of_an_include() {
    // A `#line` inside an included header must not change the
    // including file's reported line after the include returns.
    let tmp = TempDir::new().unwrap();
    write_file(
        tmp.path(),
        "gen.h",
        "#line 500 \"fake.c\"\nint from_header;\n",
    );
    let main_path = write_file(
        tmp.path(),
        "main.c",
        "#include \"gen.h\"\nint line_here = __LINE__;\n",
    );
    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main_path).unwrap();
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    // After the include, __LINE__ should be the *physical* line of
    // `int line_here = __LINE__;` in main.c — line 2.
    let mut saw_line_here = false;
    for pair in out.windows(2) {
        if matches!(&pair[0].kind, TokenKind::Identifier(s) if s == "line_here") {
            // The rest of `= 2` is an Eq + IntegerLiteral.
            continue;
        }
        if matches!(pair[0].kind, TokenKind::Equal) {
            if let TokenKind::IntegerLiteral { value, .. } = pair[1].kind {
                saw_line_here = true;
                assert_eq!(value, 2, "__LINE__ in main.c after the include should be 2");
            }
        }
    }
    assert!(saw_line_here, "did not find the `line_here = …` assignment");
}

// ---------- E. #pragma ----------

#[test]
fn pragma_message_emits_a_note_with_the_string_contents() {
    let (mut pp, _) = run("#pragma message(\"hello pragma\")\n");
    let diags = pp.take_diagnostics();
    let notes = notes_of(&diags);
    assert_eq!(notes.len(), 1, "expected one note: {diags:?}");
    assert!(
        notes[0].message.contains("hello pragma"),
        "unexpected note: {:?}",
        notes[0].message
    );
    assert!(errors_of(&diags).is_empty());
    assert!(warnings_of(&diags).is_empty());
}

#[test]
fn pragma_gcc_diagnostic_is_silently_ignored() {
    let (mut pp, _) = run("#pragma GCC diagnostic push\nint x;\n");
    let diags = pp.take_diagnostics();
    assert!(
        diags.is_empty(),
        "GCC diagnostic pragma should be silent: {diags:?}"
    );
}

#[test]
fn pragma_stdc_fp_contract_is_silently_ignored() {
    let (mut pp, _) = run("#pragma STDC FP_CONTRACT OFF\n");
    assert!(pp.take_diagnostics().is_empty());
}

#[test]
fn pragma_pack_is_silently_ignored() {
    let (mut pp, _) = run("#pragma pack(push, 4)\n");
    assert!(pp.take_diagnostics().is_empty());
}

#[test]
fn pragma_unknown_is_silently_ignored() {
    let (mut pp, _) = run("#pragma frobnicate widget quux\nint x;\n");
    assert!(pp.take_diagnostics().is_empty());
}

#[test]
fn pragma_empty_body_is_silently_ignored() {
    let (mut pp, _) = run("#pragma\n");
    assert!(pp.take_diagnostics().is_empty());
}

// ---------- F. _Pragma ----------

#[test]
fn pragma_operator_processes_message_and_emits_a_note() {
    let (mut pp, _) = run("_Pragma(\"message(\\\"hi there\\\")\")\n");
    let diags = pp.take_diagnostics();
    let notes = notes_of(&diags);
    assert_eq!(notes.len(), 1, "expected one note: {diags:?}");
    assert!(notes[0].message.contains("hi there"));
}

#[test]
fn pragma_operator_silently_ignores_unknown_pragmas() {
    let (mut pp, _) = run("_Pragma(\"GCC diagnostic push\")\nint x;\n");
    assert!(pp.take_diagnostics().is_empty());
}

#[test]
fn pragma_operator_destringises_escaped_backslashes() {
    // `_Pragma("message(\"a\\\\b\")")` — C-lexer decodes the outer
    // string to `message("a\\b")`, destringise reduces `\\` → `\`
    // to yield `message("a\b")`, and the re-lex step then treats
    // `\b` as the standard C backspace escape (0x08).  If
    // destringisation had *not* happened, the re-lex would have
    // seen `\\b` and emitted a literal `\` + `b` instead — so the
    // presence of the backspace character is the evidence that
    // destringise stripped one layer of escaping.
    let (mut pp, _) = run("_Pragma(\"message(\\\"a\\\\\\\\b\\\")\")\n");
    let diags = pp.take_diagnostics();
    let notes = notes_of(&diags);
    assert_eq!(notes.len(), 1);
    assert!(
        notes[0].message.contains("a\u{8}"),
        "unexpected note text: {:?}",
        notes[0].message
    );
}

#[test]
fn pragma_operator_in_macro_replacement_is_processed_on_expansion() {
    // A macro body that contains `_Pragma(...)` must be processed
    // exactly as if the source had written it inline.
    let (mut pp, _) = run("#define DECLS _Pragma(\"GCC diagnostic push\")\nDECLS\nint x;\n");
    let diags = pp.take_diagnostics();
    assert!(
        diags.is_empty(),
        "expected no diagnostics for GCC pragma: {diags:?}"
    );
}

#[test]
fn pragma_operator_emits_no_tokens_into_the_output_stream() {
    // Syntactically `_Pragma(...)` must evaporate — it contributes
    // no tokens at all.
    let (_, out) = run("int a;\n_Pragma(\"GCC diagnostic push\")\nint b;\n");
    let names = identifier_names(&out);
    assert!(names.contains(&"a".to_string()));
    assert!(names.contains(&"b".to_string()));
    // `_Pragma` itself must not leak through.
    assert!(!names.contains(&"_Pragma".to_string()));
}

#[test]
fn pragma_operator_rejects_non_string_argument() {
    let (mut pp, _) = run("_Pragma(42)\n");
    let diags = pp.take_diagnostics();
    assert_eq!(errors_of(&diags).len(), 1);
}

#[test]
fn pragma_operator_in_if_zero_does_not_fire() {
    // Conditional skipping is the main loop's job — it must also
    // prevent `_Pragma` from being processed in a dead branch.
    let (mut pp, _) = run("#if 0\n_Pragma(\"message(\\\"nope\\\")\")\n#endif\nint keep;\n");
    let diags = pp.take_diagnostics();
    assert!(notes_of(&diags).is_empty());
}

// ---------- G. Interaction ----------

#[test]
fn error_and_warning_can_both_appear_in_a_single_file() {
    // `#warning` should not perturb the error-tracking state; the
    // final `has_errors` reflects only `#error`.
    let (mut pp, _) = run("#warning heads up\n#error time to stop\nint x;\n");
    let diags = pp.take_diagnostics();
    assert_eq!(warnings_of(&diags).len(), 1);
    assert_eq!(errors_of(&diags).len(), 1);
    assert!(pp.has_errors());
}

#[test]
fn line_directive_coexists_with_error_span_reporting() {
    // After `#line`, a later `#error`'s diagnostic still points at
    // the `#` token — the line override affects reporting of
    // __LINE__/__FILE__, not the raw byte-offset span.
    let src = "#line 999 \"synthetic.c\"\n#error oh\n";
    let (mut pp, _) = run(src);
    let diags = pp.take_diagnostics();
    let errs = errors_of(&diags);
    assert_eq!(errs.len(), 1);
    // The `#` of `#error` is at byte offset 25 in the source above
    // (`#line 999 "synthetic.c"\n` is 24 bytes, then newline → 25).
    let hash_offset = src.find("#error").unwrap();
    assert_eq!(errs[0].span.start as usize, hash_offset);
}

#[test]
fn pragma_and_pragma_operator_share_the_message_dispatch() {
    // `#pragma message(...)` and `_Pragma("message(...)")` must
    // both end up in the same note channel.
    let (mut pp, _) = run("#pragma message(\"A\")\n_Pragma(\"message(\\\"B\\\")\")\n");
    let diags = pp.take_diagnostics();
    let notes = notes_of(&diags);
    assert_eq!(notes.len(), 2);
    assert!(notes.iter().any(|d| d.message.contains("A")));
    assert!(notes.iter().any(|d| d.message.contains("B")));
}

#[test]
fn null_directive_after_complex_directives_still_works() {
    // A regression guard for the null directive: it must survive
    // alongside the full directive set now wired up.
    let (mut pp, out) = run("#define FOO 42\n#\n#if 1\n#\n#endif\nint x = FOO;\n");
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let ks: Vec<_> = out.iter().map(|t| t.kind.clone()).collect();
    assert!(matches!(ks[0], TokenKind::Int));
    assert!(
        ks.iter()
            .any(|k| matches!(k, TokenKind::IntegerLiteral { value: 42, .. })),
        "FOO should have expanded to 42"
    );
}
