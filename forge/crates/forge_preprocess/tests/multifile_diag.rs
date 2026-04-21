//! End-to-end check that multi-file `Span` propagation works.
//!
//! Phase 2F.2 extended [`Span`] with a [`FileId`] so a token's byte
//! offset is disambiguated by the file it came from.  This test exercises
//! the plumbing:
//!
//! * a main file that `#include`s a header,
//! * the header `#define`s a macro and declares a symbol,
//! * after preprocessing we inspect each output token's `span.file`.
//!
//! The promise we verify here is:
//!
//! 1. Tokens lexed from `main.c` carry the `FileId` the source map
//!    allocated for `main.c`.
//! 2. Tokens lexed from `helper.h` (including the `42` stored inside
//!    `MY_MACRO`'s replacement list that is later spliced into `main.c`)
//!    carry the `FileId` allocated for `helper.h`.
//!
//! We do **not** yet exercise macro backtrace / expansion-site chaining —
//! that lands with Phase 2F.3.

use forge_diagnostics::{render_diagnostics_to_string, Diagnostic, ExpansionId};
use forge_lexer::{Lexer, TokenKind};
use forge_preprocess::{FileId, PreprocessConfig, Preprocessor};

/// Drop a file at `dir/name` with `contents`, returning its absolute path.
fn write_file(dir: &std::path::Path, name: &str, contents: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, contents).unwrap();
    p
}

/// Find the [`forge_diagnostics::FileId`] of the file whose registered
/// name *ends with* `suffix`.  Useful because the preprocessor stores
/// canonicalised absolute paths but we only know the basename here.
fn file_id_for_suffix(
    sm: &forge_diagnostics::SourceMap,
    suffix: &str,
) -> forge_diagnostics::FileId {
    sm.iter()
        .find(|f| f.name.ends_with(suffix))
        .unwrap_or_else(|| panic!("no registered file ends with {suffix:?}"))
        .id
}

#[test]
fn tokens_carry_the_file_id_of_the_file_they_were_lexed_from() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_file(
        tmp.path(),
        "helper.h",
        "#define MY_MACRO 42\nint header_decl;\n",
    );
    let main_path = write_file(
        tmp.path(),
        "main.c",
        "#include \"helper.h\"\nint x = MY_MACRO;\nint after = 1;\n",
    );

    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let tokens = pp.run_file(&main_path).unwrap();
    let diags = pp.take_diagnostics();
    assert!(
        !diags
            .iter()
            .any(|d| matches!(d.severity, forge_diagnostics::Severity::Error)),
        "unexpected diagnostics: {diags:?}"
    );

    let sm = pp.source_map();
    let main_id = file_id_for_suffix(sm, "main.c");
    let header_id = file_id_for_suffix(sm, "helper.h");
    assert_ne!(
        main_id, header_id,
        "main.c and helper.h must have distinct FileIds"
    );

    // Find concrete markers in the output token stream and check their
    // `span.file`.  We pick unambiguous anchors:
    //   * `header_decl` — an identifier that only appears verbatim in
    //     helper.h.
    //   * `after`       — an identifier that only appears verbatim in
    //     main.c.
    //   * `42`          — the sole integer literal; it was lexed as part
    //     of MY_MACRO's replacement list inside helper.h, so even though
    //     it lands in main.c's token stream it must still carry helper.h's
    //     FileId.
    //   * `x`, `1`      — other main.c tokens surrounding the expansion.
    let find_ident = |name: &str| {
        tokens
            .iter()
            .find(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == name))
            .unwrap_or_else(|| panic!("identifier {name:?} not found in output"))
    };

    let header_decl_tok = find_ident("header_decl");
    assert_eq!(
        header_decl_tok.span.file, header_id,
        "`header_decl` was declared in helper.h, its span.file must match helper.h's FileId"
    );

    let after_tok = find_ident("after");
    assert_eq!(
        after_tok.span.file, main_id,
        "`after` was written in main.c, its span.file must match main.c's FileId"
    );

    let x_tok = find_ident("x");
    assert_eq!(
        x_tok.span.file, main_id,
        "`x` was written in main.c, its span.file must match main.c's FileId"
    );

    // Only one integer literal in the program: MY_MACRO's `42`.  It was
    // lexed from helper.h; it must keep that `FileId` even after being
    // spliced into main.c's stream by macro expansion.
    let forty_two = tokens
        .iter()
        .find(|t| matches!(t.kind, TokenKind::IntegerLiteral { value: 42, .. }))
        .expect("expected the `42` literal from MY_MACRO");
    assert_eq!(
        forty_two.span.file, header_id,
        "`42` was lexed from helper.h as part of MY_MACRO's replacement \
         list — its span.file must still point at helper.h after expansion"
    );

    // The trailing `1` in `int after = 1;` is a main.c literal; use it to
    // sanity-check that not every integer literal inherits helper.h's id.
    let one = tokens
        .iter()
        .find(|t| matches!(t.kind, TokenKind::IntegerLiteral { value: 1, .. }))
        .expect("expected the `1` literal from main.c");
    assert_eq!(
        one.span.file, main_id,
        "`1` was written in main.c, its span.file must match main.c's FileId"
    );
}

// ---------------------------------------------------------------------------
// Phase 2F.3 — macro-expansion backtrace (`Span::expanded_from`)
// ---------------------------------------------------------------------------

/// Preprocess `src` against a default config from an in-memory buffer.
/// Returns the preprocessor (so callers can inspect `expansions()` and
/// `source_map()` before it is dropped) and the produced token stream.
fn run_inline(src: &str) -> (Preprocessor, Vec<forge_lexer::Token>) {
    let tokens = Lexer::new(src, FileId::PRIMARY).tokenize();
    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_with_source(tokens, src, "<input>");
    let diags = pp.take_diagnostics();
    assert!(
        !diags
            .iter()
            .any(|d| matches!(d.severity, forge_diagnostics::Severity::Error)),
        "unexpected diagnostics: {diags:?}"
    );
    (pp, out)
}

/// Single-level object-like expansion: every token produced by expanding
/// `PI` carries a fresh `ExpansionId`, and the corresponding frame names
/// the macro.
#[test]
fn macro_expansion_single_level() {
    let (pp, tokens) = run_inline("#define PI 314\nint x = PI;\n");

    let three_fourteen = tokens
        .iter()
        .find(|t| matches!(t.kind, TokenKind::IntegerLiteral { value: 314, .. }))
        .expect("expected the `314` literal from PI's expansion");

    assert!(
        three_fourteen.span.expanded_from.is_some(),
        "token produced by macro expansion must carry an ExpansionId, got NONE"
    );

    let frame = pp
        .expansions()
        .get(three_fourteen.span.expanded_from)
        .expect("expansion id must resolve to a frame");
    assert_eq!(frame.macro_name, "PI");
    assert_eq!(
        frame.parent,
        ExpansionId::NONE,
        "a top-level expansion's parent should be NONE (no outer macro)"
    );
}

/// Two-level nested expansion: `A(x)` expands to `B(x)`, which expands
/// to `(x + 1)`.  The produced `+` token carries B's expansion id, and
/// walking the backtrace yields `[B, A]`.
#[test]
fn macro_expansion_nested_two_levels() {
    let src = "#define B(x) (x + 1)\n#define A(x) B(x)\nint y = A(7);\n";
    let (pp, tokens) = run_inline(src);

    let plus = tokens
        .iter()
        .find(|t| matches!(t.kind, TokenKind::Plus))
        .expect("expected `+` from B's body after A(7) expansion");

    assert!(
        plus.span.expanded_from.is_some(),
        "`+` came from macro expansion and must carry an ExpansionId"
    );

    let chain: Vec<&str> = pp
        .expansions()
        .backtrace(plus.span.expanded_from)
        .iter()
        .map(|f| f.macro_name.as_str())
        .collect();
    assert_eq!(
        chain,
        vec!["B", "A"],
        "expected innermost-first backtrace B -> A, got {chain:?}"
    );
}

/// C17 §6.10.3.1 argument preservation: tokens that came from an inner
/// expansion retain that inner id when substituted into an outer macro's
/// body.  With `ID(x)=x` and `F(x)=(x*2)`, the `3` in `F(ID(3))` keeps
/// ID's expansion id — it does NOT get F's.
#[test]
fn macro_argument_preservation() {
    let src = "#define ID(x) x\n#define F(x) (x*2)\nint r = F(ID(3));\n";
    let (pp, tokens) = run_inline(src);

    let three = tokens
        .iter()
        .find(|t| matches!(t.kind, TokenKind::IntegerLiteral { value: 3, .. }))
        .expect("expected the `3` literal");
    let two = tokens
        .iter()
        .find(|t| matches!(t.kind, TokenKind::IntegerLiteral { value: 2, .. }))
        .expect("expected the `2` literal (from F's body)");

    assert!(
        three.span.expanded_from.is_some(),
        "`3` came through ID's expansion and must carry an ExpansionId"
    );
    assert!(
        two.span.expanded_from.is_some(),
        "`2` came from F's body and must carry an ExpansionId"
    );

    let three_frame = pp
        .expansions()
        .get(three.span.expanded_from)
        .expect("resolvable");
    let two_frame = pp
        .expansions()
        .get(two.span.expanded_from)
        .expect("resolvable");

    assert_eq!(
        three_frame.macro_name, "ID",
        "`3` must be attributed to ID (argument preservation), got {}",
        three_frame.macro_name
    );
    assert_eq!(
        two_frame.macro_name, "F",
        "`2` is a body token of F, got {}",
        two_frame.macro_name
    );
}

/// The magic `__LINE__` macro also records an expansion frame so
/// diagnostics against synthesized built-in tokens can still render a
/// backtrace.
#[test]
fn builtin_line_expansion_tracked() {
    let (pp, tokens) = run_inline("int line = __LINE__;\n");

    let int_lit = tokens
        .iter()
        .find(|t| matches!(t.kind, TokenKind::IntegerLiteral { .. }))
        .expect("__LINE__ must produce one IntegerLiteral");

    assert!(
        int_lit.span.expanded_from.is_some(),
        "__LINE__ replacement token must carry an ExpansionId"
    );
    let frame = pp
        .expansions()
        .get(int_lit.span.expanded_from)
        .expect("resolvable");
    assert_eq!(frame.macro_name, "__LINE__");
}

/// End-to-end: a diagnostic anchored on an expansion-produced token
/// renders with one auxiliary "in expansion of macro 'X'" label per
/// frame in the backtrace.
#[test]
fn diagnostic_emits_macro_backtrace() {
    let src = "#define INNER 99\n#define OUTER INNER\nint z = OUTER;\n";
    let (pp, tokens) = run_inline(src);

    let ninety_nine = tokens
        .iter()
        .find(|t| matches!(t.kind, TokenKind::IntegerLiteral { value: 99, .. }))
        .expect("expected the `99` literal from INNER via OUTER");

    assert!(
        ninety_nine.span.expanded_from.is_some(),
        "`99` must carry an expansion id"
    );

    let diag = Diagnostic::error("synthetic probe")
        .span(ninety_nine.span)
        .label("here");
    let rendered = render_diagnostics_to_string(pp.source_map(), pp.expansions(), &[diag]);

    assert!(
        rendered.contains("in expansion of macro 'INNER'"),
        "expected innermost frame INNER in rendered output, got:\n{rendered}"
    );
    assert!(
        rendered.contains("in expansion of macro 'OUTER'"),
        "expected outer frame OUTER in rendered output, got:\n{rendered}"
    );
}
