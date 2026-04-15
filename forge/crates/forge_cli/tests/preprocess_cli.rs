//! End-to-end integration test for `forge -E`.
//!
//! Spawns the compiled `forge` binary as a subprocess, feeds it a tiny C
//! source file via a temp path, and asserts that stdout contains the
//! preprocessed program with every `#define` expanded and every `-D`
//! flag honoured.
//!
//! The `-I` path is derived from the temp directory so the included
//! header is resolved by the real file-system search logic rather than
//! by in-memory shortcuts.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Absolute path to the compiled `forge` binary, provided by Cargo.
const FORGE_BIN: &str = env!("CARGO_BIN_EXE_forge");

/// A throwaway directory under `std::env::temp_dir()` that is removed on
/// drop, best-effort.  We pick a process-id-and-test-name-suffixed name
/// so concurrent test runs don't collide.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "forge_preprocess_cli_{}_{}_{}",
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

#[test]
fn forge_dash_e_expands_cli_define_and_quote_include() {
    let tmp = TempDir::new("happy");

    // Header that the main source will `#include "..."`.
    tmp.file(
        "values.h",
        "#define HEADER_VAL 55\n#define DOUBLE(x) ((x) + (x))\n",
    );

    // Main source: exercises `-D`, `-U`, quote include, and a function
    // macro pulled from the header.
    let main = tmp.file(
        "main.c",
        r#"#include "values.h"
int v = CUSTOM_VAL;
int h = HEADER_VAL;
int d = DOUBLE(7);
#ifdef LEGACY
int legacy_marker;
#else
int modern_marker = 1;
#endif
"#,
    );

    let output = Command::new(FORGE_BIN)
        .arg("-E")
        .arg("-D")
        .arg("CUSTOM_VAL=999")
        .arg("-U")
        .arg("LEGACY")
        .arg(&main)
        .output()
        .expect("spawn forge");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "forge -E exited with {}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        output.status,
    );
    assert!(
        stdout.contains("int v = 999;"),
        "expected CLI -D expansion in stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("int h = 55;"),
        "expected header macro expansion in stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("int d = ((7) + (7));"),
        "expected function-like header macro expansion in stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("int modern_marker = 1;"),
        "expected -U LEGACY to take the else-branch:\n{stdout}"
    );
    assert!(
        !stdout.contains('#'),
        "no preprocessing directives should leak into stdout:\n{stdout}"
    );
}
