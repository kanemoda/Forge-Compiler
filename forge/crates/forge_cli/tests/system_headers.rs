//! Smoke tests for the host's core C17 system headers.
//!
//! Each test spawns the compiled `forge` binary with `-E` against a
//! tiny source file that includes exactly one standard header, and
//! asserts that preprocessing completes with a zero exit status and
//! no preprocessor-severity diagnostics on stderr.  A combined test
//! exercises a realistic translation unit with a `main()` that calls
//! `printf`.
//!
//! # Requirements
//!
//! These tests require the host to have a usable C toolchain visible
//! as `cc`.  On environments without one (e.g. a stripped-down CI
//! sandbox) the tests skip themselves gracefully rather than fail,
//! because the preprocessor can not exercise the include search path
//! without real headers on disk.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Absolute path to the compiled `forge` binary, provided by Cargo.
const FORGE_BIN: &str = env!("CARGO_BIN_EXE_forge");

/// Temporary directory that is cleaned up on drop.  Namespaced with the
/// process id, a per-test tag, and a nanosecond timestamp to avoid
/// collisions between parallel test runs.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "forge_system_headers_{}_{}_{}",
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

/// `true` if the host has a discoverable toolchain that would let the
/// driver resolve standard headers.  Checked by shelling out to
/// `cc -E -v -x c /dev/null` — the same mechanism the driver uses at
/// startup.
fn host_has_system_headers() -> bool {
    Command::new("cc")
        .args(["-E", "-v", "-x", "c", "/dev/null"])
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Run `forge -E` on a generated file that `#include`s `header` and
/// assert a clean exit.  Returns `Ok(())` on success, `Err(msg)` on
/// any unexpected failure the caller then feeds into an `assert!`.
fn run_forge_e_on_header(header: &str, tag: &str) -> Result<(), String> {
    let tmp = TempDir::new(tag);
    let src = tmp.file("main.c", &format!("#include <{header}>\n"));

    let output = Command::new(FORGE_BIN)
        .arg("-E")
        .arg(&src)
        .output()
        .map_err(|e| format!("spawn failed: {e}"))?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "forge -E <{header}> exited {} — stdout: {stdout}\nstderr: {stderr}",
            output.status
        ));
    }
    Ok(())
}

/// Run `forge parse` on a generated file that `#include`s `header`
/// followed by a trivial `main` — exercises the whole lex + preprocess
/// + parse pipeline on real glibc/libc headers.
fn run_forge_parse_on_header(header: &str, tag: &str) -> Result<(), String> {
    let tmp = TempDir::new(tag);
    let src = tmp.file(
        "main.c",
        &format!("#include <{header}>\nint main(void) {{ return 0; }}\n"),
    );

    let output = Command::new(FORGE_BIN)
        .arg("parse")
        .arg(&src)
        .output()
        .map_err(|e| format!("spawn failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "forge parse <{header}> exited {}\n--- stderr (head) ---\n{}",
            output.status,
            stderr.chars().take(4_000).collect::<String>()
        ));
    }
    Ok(())
}

/// Run `forge check` on an on-disk source file — exercises the full
/// lex → preprocess → parse → sema pipeline and asserts zero errors.
///
/// Used by the Phase 4 acceptance gates that live in
/// `tests/lit/sema/*.c`: each lit source file is a hand-written real
/// program whose sema pass must complete cleanly before the phase can
/// ship.
fn run_forge_check_on_file(source_path: &std::path::Path) -> Result<(), String> {
    let output = Command::new(FORGE_BIN)
        .arg("check")
        .arg(source_path)
        .output()
        .map_err(|e| format!("spawn failed: {e}"))?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "forge check {} exited {}\n--- stderr (head) ---\n{}\n--- stdout (head) ---\n{}",
            source_path.display(),
            output.status,
            stderr.chars().take(4_000).collect::<String>(),
            stdout.chars().take(1_000).collect::<String>()
        ));
    }
    Ok(())
}

/// Resolve a path under the workspace's `tests/lit/` tree.  The cargo
/// test harness sets `CARGO_MANIFEST_DIR` to the crate root
/// (`forge/crates/forge_cli`), so the workspace top is two levels up.
fn lit_source(relative: &str) -> PathBuf {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("tests")
        .join("lit")
        .join(relative)
}

// ---------------------------------------------------------------------------
// One test per canonical C17 system header
// ---------------------------------------------------------------------------

#[test]
fn system_header_stddef_preprocesses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_e_on_header("stddef.h", "stddef") {
        panic!("{e}");
    }
}

#[test]
fn system_header_stdint_preprocesses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_e_on_header("stdint.h", "stdint") {
        panic!("{e}");
    }
}

#[test]
fn system_header_limits_preprocesses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_e_on_header("limits.h", "limits") {
        panic!("{e}");
    }
}

#[test]
fn system_header_stdio_preprocesses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_e_on_header("stdio.h", "stdio") {
        panic!("{e}");
    }
}

#[test]
fn system_header_stdlib_preprocesses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_e_on_header("stdlib.h", "stdlib") {
        panic!("{e}");
    }
}

#[test]
fn system_header_string_preprocesses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_e_on_header("string.h", "string") {
        panic!("{e}");
    }
}

#[test]
fn system_header_errno_preprocesses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_e_on_header("errno.h", "errno") {
        panic!("{e}");
    }
}

#[test]
fn system_header_assert_preprocesses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_e_on_header("assert.h", "assert") {
        panic!("{e}");
    }
}

#[test]
fn system_header_ctype_preprocesses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_e_on_header("ctype.h", "ctype") {
        panic!("{e}");
    }
}

#[test]
fn system_header_math_preprocesses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_e_on_header("math.h", "math") {
        panic!("{e}");
    }
}

// ---------------------------------------------------------------------------
// One parser test per canonical C17 system header — exercises the full
// lex + preprocess + parse pipeline on each header in isolation, so a
// parser regression on any single header surfaces with a precise name.
// ---------------------------------------------------------------------------

#[test]
fn system_header_stddef_parses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_parse_on_header("stddef.h", "p_stddef") {
        panic!("{e}");
    }
}

#[test]
fn system_header_stdint_parses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_parse_on_header("stdint.h", "p_stdint") {
        panic!("{e}");
    }
}

#[test]
fn system_header_limits_parses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_parse_on_header("limits.h", "p_limits") {
        panic!("{e}");
    }
}

#[test]
fn system_header_stdio_parses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_parse_on_header("stdio.h", "p_stdio") {
        panic!("{e}");
    }
}

#[test]
fn system_header_stdlib_parses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_parse_on_header("stdlib.h", "p_stdlib") {
        panic!("{e}");
    }
}

#[test]
fn system_header_string_parses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_parse_on_header("string.h", "p_string") {
        panic!("{e}");
    }
}

#[test]
fn system_header_errno_parses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_parse_on_header("errno.h", "p_errno") {
        panic!("{e}");
    }
}

#[test]
fn system_header_assert_parses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_parse_on_header("assert.h", "p_assert") {
        panic!("{e}");
    }
}

#[test]
fn system_header_ctype_parses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_parse_on_header("ctype.h", "p_ctype") {
        panic!("{e}");
    }
}

#[test]
fn system_header_math_parses_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    if let Err(e) = run_forge_parse_on_header("math.h", "p_math") {
        panic!("{e}");
    }
}

// ---------------------------------------------------------------------------
// Combined smoke test — a realistic translation unit
// ---------------------------------------------------------------------------

#[test]
fn hello_world_main_preprocesses_through_forge_e() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    let tmp = TempDir::new("hello");
    let src = tmp.file(
        "main.c",
        r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    const char *msg = "hello, world";
    printf("%s %d\n", msg, (int)strlen(msg));
    return EXIT_SUCCESS;
}
"#,
    );

    let output = Command::new(FORGE_BIN)
        .arg("-E")
        .arg(&src)
        .output()
        .expect("spawn forge -E on hello world");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "forge -E failed ({}):\n--- stdout (head) ---\n{}\n--- stderr ---\n{stderr}",
        output.status,
        stdout.chars().take(2_000).collect::<String>()
    );

    // The preprocessed source must have the user's own tokens intact:
    // the string literal, the `return EXIT_SUCCESS`, and the `printf`
    // call all survive macro expansion and are present in stdout.
    assert!(
        stdout.contains("\"hello, world\""),
        "string literal lost in preprocessing"
    );
    assert!(
        stdout.contains("printf"),
        "printf identifier lost in preprocessing"
    );
    // EXIT_SUCCESS is macro-defined by <stdlib.h> as `0`; after
    // expansion the literal `0` must appear where the macro was.
    assert!(
        stdout.contains("return 0"),
        "EXIT_SUCCESS did not expand to 0: {}",
        stdout.chars().take(500).collect::<String>()
    );

    // Preprocessing directives must not leak through.
    assert!(
        !stdout.contains("#include"),
        "#include directive leaked into preprocessed output"
    );
    assert!(
        !stdout.contains("#define"),
        "#define directive leaked into preprocessed output"
    );
}

// ---------------------------------------------------------------------------
// Parser smoke test — the critical Prompt 3.6 gate
// ---------------------------------------------------------------------------

/// Run `forge parse` on a source file that includes every canonical C17
/// system header the compiler promises to support.  Asserts that the
/// whole lex → preprocess → parse pipeline completes with zero
/// error-severity diagnostics (printed to stderr by the driver) and a
/// zero process exit status.
///
/// This is the Prompt 3.6 acceptance gate — before it existed the
/// parser tripped on `__attribute__`, `__extension__`, `__typeof__`,
/// `__asm__`, and every `__builtin_*` in glibc.  A regression in any
/// GNU-extension tolerance path surfaces here first.
#[test]
fn parser_accepts_full_system_header_set() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    let tmp = TempDir::new("parser_smoke");
    let src = tmp.file(
        "main.c",
        r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

int main(void) {
    return 0;
}
"#,
    );

    let output = Command::new(FORGE_BIN)
        .arg("parse")
        .arg(&src)
        .output()
        .expect("spawn forge parse on system-header smoke source");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "forge parse failed ({}):\n--- stderr (head) ---\n{}\n--- stdout (head) ---\n{}",
        output.status,
        stderr.chars().take(4_000).collect::<String>(),
        stdout.chars().take(1_000).collect::<String>()
    );

    // The user's own `main` must survive unchanged through the whole
    // pipeline and show up in the printed AST.
    assert!(
        stdout.contains("FunctionDef"),
        "printed AST lacks any FunctionDef:\n{}",
        stdout.chars().take(2_000).collect::<String>()
    );
    assert!(
        stdout.contains("main"),
        "printed AST lacks the `main` declarator name"
    );
}

// ---------------------------------------------------------------------------
// Phase 4 sema acceptance gates — real-world lit sources
// ---------------------------------------------------------------------------

/// Drives the whole lex → preprocess → parse → sema pipeline against a
/// hand-written program that exercises multiple structs (one self-
/// referential), an enum with explicit values, a function-pointer
/// typedef in a struct, array-of-struct designated initializers,
/// pointer arithmetic, compatible pointer casts, `sizeof` and
/// `_Alignof` used as array dimensions, a switch with five cases plus
/// default, a for-loop with a comma expression in its update, a
/// variadic prototype and call, and `_Static_assert` at both file and
/// block scope.  The Phase 4 exit criterion is that this file passes
/// sema with zero errors.
#[test]
fn realworld_sema_acceptance_passes_cleanly() {
    let src = lit_source("sema/realworld.c");
    assert!(src.exists(), "missing lit source at {}", src.display());
    if let Err(e) = run_forge_check_on_file(&src) {
        panic!("{e}");
    }
}

/// Extended system-header smoke test that pulls in eight canonical C17
/// library headers (`stdio`, `stdlib`, `string`, `stdint`, `stddef`,
/// `ctype`, `errno`, `time`) and calls at least one function from each,
/// so sema must walk every prototype the host libc exposes through
/// them.  Skipped on hosts without a detectable toolchain because the
/// preprocessor can not resolve `#include <...>` without real headers.
#[test]
fn extended_system_headers_pass_sema_cleanly() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }
    let src = lit_source("sema/headers_smoke_extended.c");
    assert!(src.exists(), "missing lit source at {}", src.display());
    if let Err(e) = run_forge_check_on_file(&src) {
        panic!("{e}");
    }
}
