//! File-driven "lit-style" test runner for the Forge compiler.
//!
//! Test files live under `tests/lit/` at the workspace root. Each `.c` file
//! carries embedded directives in comments:
//!
//! ```text
//! // RUN: forge check %s
//! // CHECK: expected output substring
//! // ERROR: expected stderr substring
//! ```
//!
//! **Directive reference**
//!
//! | Directive | Meaning |
//! |-----------|---------|
//! | `// RUN: <cmd>` | Run this command. `%s` expands to the test file's absolute path. `forge` at argv[0] is replaced with the compiled binary under test. |
//! | `// CHECK: <str>` | `<str>` must appear as a substring of stdout. |
//! | `// ERROR: <str>` | `<str>` must appear as a substring of stderr. |
//!
//! A test passes when every RUN command exits 0 and every CHECK/ERROR
//! pattern is found in the corresponding output stream. A file with no
//! `// RUN:` directive is an unconditional failure.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use libtest_mimic::{Arguments, Failed, Trial};

/// Absolute path to the compiled `forge` binary, provided by Cargo.
///
/// Cargo sets `CARGO_BIN_EXE_<name>` for every binary defined in the same
/// package, ensuring the binary is built before any integration test runs.
const FORGE_BIN: &str = env!("CARGO_BIN_EXE_forge");

fn main() {
    let args = Arguments::from_args();
    let tests = collect_tests().unwrap_or_else(|e| {
        eprintln!("error: failed to collect lit tests: {e}");
        std::process::exit(1);
    });
    libtest_mimic::run(&args, tests).exit();
}

// ---------------------------------------------------------------------------
// Test discovery
// ---------------------------------------------------------------------------

/// Returns the `tests/lit/` directory at the workspace root.
fn lit_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is .../forge/crates/forge_cli at compile time.
    // Two levels up is the workspace root (.../forge/).
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("lit")
}

/// Walk `tests/lit/` recursively and create one [`Trial`] per `.c` file.
fn collect_tests() -> Result<Vec<Trial>, Box<dyn std::error::Error>> {
    let root = lit_dir();
    let mut trials = Vec::new();
    visit_dir(&root, &root, &mut trials)?;
    // Sort for deterministic output regardless of filesystem order.
    trials.sort_by(|a, b| a.name().cmp(b.name()));
    Ok(trials)
}

fn visit_dir(
    root: &Path,
    dir: &Path,
    trials: &mut Vec<Trial>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            visit_dir(root, &path, trials)?;
        } else if path.extension().and_then(OsStr::to_str) == Some("c") {
            // Use a path relative to the lit root as the test name so output
            // stays readable regardless of where the workspace lives on disk.
            let name = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/"); // normalise path separators on Windows
            let abs_path = path.canonicalize().unwrap_or(path);
            trials.push(Trial::test(name, move || run_lit_test(&abs_path)));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Directive parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Directives {
    run: Vec<String>,
    checks: Vec<String>,
    errors: Vec<String>,
}

fn parse_directives(source: &str) -> Directives {
    let mut d = Directives::default();
    for line in source.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("// RUN:") {
            d.run.push(rest.trim().to_string());
        } else if let Some(rest) = t.strip_prefix("// CHECK:") {
            d.checks.push(rest.trim().to_string());
        } else if let Some(rest) = t.strip_prefix("// ERROR:") {
            d.errors.push(rest.trim().to_string());
        }
    }
    d
}

// ---------------------------------------------------------------------------
// Test execution
// ---------------------------------------------------------------------------

fn run_lit_test(path: &Path) -> Result<(), Failed> {
    let source = fs::read_to_string(path)
        .map_err(|e| Failed::from(format!("cannot read {}: {e}", path.display())))?;

    let directives = parse_directives(&source);

    if directives.run.is_empty() {
        return Err(Failed::from("no // RUN: directive found in test file"));
    }

    for run_line in &directives.run {
        execute_run(run_line, path, &directives.checks, &directives.errors)?;
    }

    Ok(())
}

/// Execute a single RUN command and verify CHECK/ERROR patterns.
fn execute_run(
    run_line: &str,
    test_file: &Path,
    checks: &[String],
    errors: &[String],
) -> Result<(), Failed> {
    // Expand %s → absolute path to the test file.
    let cmd_str = run_line.replace("%s", &test_file.to_string_lossy());

    // Minimal argv split (no quoting needed for our test suite style).
    let mut words = cmd_str.split_whitespace();
    let bin_str = words
        .next()
        .ok_or_else(|| Failed::from("empty RUN command"))?;
    let argv_rest: Vec<&str> = words.collect();

    // Map the literal word "forge" to the binary under test; pass other
    // executables (e.g., system tools) through as-is.
    let binary: &Path = if bin_str == "forge" {
        Path::new(FORGE_BIN)
    } else {
        Path::new(bin_str)
    };

    let output = Command::new(binary)
        .args(&argv_rest)
        .output()
        .map_err(|e| Failed::from(format!("failed to spawn `{}`: {e}", binary.display())))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        return Err(Failed::from(format!(
            "command exited with {status}\n\
             --- stdout ---\n{stdout}\
             --- stderr ---\n{stderr}",
            status = output.status,
        )));
    }

    for check in checks {
        if !stdout.contains(check.as_str()) {
            return Err(Failed::from(format!(
                "CHECK `{check}` not found in stdout:\n{stdout}"
            )));
        }
    }

    for error in errors {
        if !stderr.contains(error.as_str()) {
            return Err(Failed::from(format!(
                "ERROR `{error}` not found in stderr:\n{stderr}"
            )));
        }
    }

    Ok(())
}
