//! Phase 2 multi-file Span fix — regression guards.
//!
//! These tests verify two end-to-end behaviours that used to break (or
//! could break again if the Span plumbing regresses):
//!
//! 1. A type error on a user-written line must report against the user
//!    file, never against a `#include`d system header — even though
//!    sema sees header declarations in the same translation unit.
//! 2. A type error produced by expanding a macro defined in a header
//!    must render with "in expansion of macro 'X'" labels so the user
//!    can see both where the macro was invoked and where it was
//!    defined.

use std::fs;
use std::path::PathBuf;

use forge_diagnostics::{render_diagnostics_to_string, Severity};
use forge_driver::{compile, CompileOptions};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "forge_multifile_regression_{}_{}_{}",
            std::process::id(),
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir { path }
    }

    fn file(&self, name: &str, contents: &str) -> PathBuf {
        let p = self.path.join(name);
        fs::write(&p, contents).expect("write temp file");
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A type error on a user-written line must render with the user file's
/// name — not any `#include`d system header name.  `int x = "hello";`
/// is caught by sema as an assignment-from-incompatible-type error; the
/// offending token was written in `user.c`, so that is where the
/// diagnostic must point.
#[test]
fn regression_user_error_reports_user_file() {
    let tmp = TempDir::new("user_error");
    let user_path = tmp.file(
        "user.c",
        "#include <stdio.h>\n\
         int main(void) {\n\
             int x = \"hello\";\n\
             (void)x;\n\
             return 0;\n\
         }\n",
    );
    let user_name = user_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let out = compile(
        user_path.to_str().unwrap(),
        &fs::read_to_string(&user_path).unwrap(),
        &CompileOptions::default(),
    );

    // At least one error-level diagnostic.
    let error = out
        .diagnostics
        .iter()
        .find(|d| matches!(d.severity, Severity::Error))
        .expect("expected an error-level diagnostic for int = \"hello\"");

    // The primary span must belong to the user file, NOT stdio.h (or
    // any other header).  We verify this by looking up the span's
    // FileId in the SourceMap and checking the registered name.
    let file = out
        .source_map
        .get(error.span.file)
        .expect("diagnostic span must point to a registered source file");
    assert!(
        file.name.ends_with(&user_name) || file.name.ends_with("user.c"),
        "diagnostic must report against the user file, got {:?}",
        file.name
    );
    assert!(
        !file.name.contains("stdio.h"),
        "diagnostic must not be attributed to stdio.h, got {:?}",
        file.name
    );

    // Rendered output mentions the user file and NOT stdio.h as the
    // primary location for this diagnostic.
    let rendered = render_diagnostics_to_string(
        &out.source_map,
        &out.expansions,
        std::slice::from_ref(error),
    );
    assert!(
        rendered.contains(&user_name) || rendered.contains("user.c"),
        "rendered output should mention the user file, got:\n{rendered}"
    );
}

/// A type error produced by a macro defined in a header must render
/// with "in expansion of macro 'X'" labels on the invocation site.
/// Exercises Phase 2F.3 plumbing end-to-end: the `"hello"` token that
/// lands in `main()` has `expanded_from` stamped by the preprocessor,
/// and the renderer walks the expansion table to surface that chain.
#[test]
fn regression_macro_error_emits_backtrace() {
    let tmp = TempDir::new("macro_error");
    tmp.file("badmacro.h", "#define BAD_INIT \"hello\"\n");
    let user_path = tmp.file(
        "user.c",
        "#include \"badmacro.h\"\n\
         int main(void) {\n\
             int x = BAD_INIT;\n\
             (void)x;\n\
             return 0;\n\
         }\n",
    );

    let out = compile(
        user_path.to_str().unwrap(),
        &fs::read_to_string(&user_path).unwrap(),
        &CompileOptions::default(),
    );

    let error = out
        .diagnostics
        .iter()
        .find(|d| matches!(d.severity, Severity::Error))
        .expect("expected an error on int = BAD_INIT");

    // The token's span must carry an expansion id.
    assert!(
        error.span.expanded_from.is_some(),
        "macro-produced token must carry an ExpansionId (span {:?})",
        error.span
    );

    // Walking the backtrace yields the BAD_INIT frame.
    let chain: Vec<&str> = out
        .expansions
        .backtrace(error.span.expanded_from)
        .iter()
        .map(|f| f.macro_name.as_str())
        .collect();
    assert!(
        chain.contains(&"BAD_INIT"),
        "backtrace must contain BAD_INIT, got {chain:?}"
    );

    // End-to-end: rendered diagnostic contains "in expansion of macro
    // 'BAD_INIT'" (the invocation-site label).
    let rendered = render_diagnostics_to_string(
        &out.source_map,
        &out.expansions,
        std::slice::from_ref(error),
    );
    assert!(
        rendered.contains("in expansion of macro 'BAD_INIT'"),
        "rendered output must contain the macro-backtrace label, got:\n{rendered}"
    );
}
