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

use forge_lexer::TokenKind;
use forge_preprocess::{PreprocessConfig, Preprocessor};

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
