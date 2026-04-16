//! `#include` resolution, pragma once, include guards, and path ordering.

use std::fs;

use forge_diagnostics::Severity;
use forge_lexer::TokenKind;
use tempfile::TempDir;

use super::helpers::*;
use crate::system_includes::detect_system_include_paths;
use crate::{PreprocessConfig, Preprocessor};

// Test A — `#include "local.h"` pulls in tokens from the same directory.
#[test]
fn include_quote_form_loads_a_local_header() {
    let tmp = TempDir::new().unwrap();
    write_file(tmp.path(), "local.h", "int local_marker;\n");
    let main_path = write_file(
        tmp.path(),
        "main.c",
        "#include \"local.h\"\nint main_marker;\n",
    );

    let source = fs::read_to_string(&main_path).unwrap();
    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main_path).unwrap();
    let diags = pp.take_diagnostics();
    assert!(diags.is_empty_or_no_errors(), "diags: {diags:?}");
    let names = identifier_names(&out);
    assert!(names.contains(&"local_marker".to_string()));
    assert!(names.contains(&"main_marker".to_string()));
    let _ = source; // keep clippy happy for the unused read
}

// Test B — one header `#include`s another.
#[test]
fn include_nested_header_chains_through_two_files() {
    let tmp = TempDir::new().unwrap();
    write_file(tmp.path(), "inner.h", "int inner_marker;\n");
    write_file(
        tmp.path(),
        "outer.h",
        "#include \"inner.h\"\nint outer_marker;\n",
    );
    let main_path = write_file(tmp.path(), "main.c", "#include \"outer.h\"\n");

    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main_path).unwrap();
    let diags = pp.take_diagnostics();
    assert!(diags.is_empty_or_no_errors(), "diags: {diags:?}");
    let names = identifier_names(&out);
    assert!(names.contains(&"inner_marker".to_string()));
    assert!(names.contains(&"outer_marker".to_string()));
}

// Test C — a quote include resolves relative to the *including* file,
// not the translation unit root.
#[test]
fn include_quote_relative_is_resolved_from_including_file_directory() {
    let tmp = TempDir::new().unwrap();
    write_file(tmp.path(), "sub/leaf.h", "int leaf_marker;\n");
    // outer.h lives in `sub/` and includes `leaf.h` via a bare name.
    write_file(tmp.path(), "sub/outer.h", "#include \"leaf.h\"\n");
    let main_path = write_file(tmp.path(), "main.c", "#include \"sub/outer.h\"\n");

    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main_path).unwrap();
    let diags = pp.take_diagnostics();
    assert!(diags.is_empty_or_no_errors(), "diags: {diags:?}");
    let names = identifier_names(&out);
    assert!(names.contains(&"leaf_marker".to_string()));
}

// Test D — `#pragma once` prevents a second copy of the header's body.
#[test]
fn pragma_once_elides_second_include() {
    let tmp = TempDir::new().unwrap();
    write_file(tmp.path(), "once.h", "#pragma once\nint once_marker;\n");
    let main_path = write_file(
        tmp.path(),
        "main.c",
        "#include \"once.h\"\n#include \"once.h\"\n",
    );

    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main_path).unwrap();
    let diags = pp.take_diagnostics();
    assert!(diags.is_empty_or_no_errors(), "diags: {diags:?}");
    let marker_count = identifier_names(&out)
        .iter()
        .filter(|s| *s == "once_marker")
        .count();
    assert_eq!(marker_count, 1, "pragma once should fire only once");
}

// Test E — a conventional `#ifndef/#define/.../#endif` guard is
// recognised as `#pragma once`-ish, so a second include is silent.
#[test]
fn canonical_include_guard_is_detected_and_second_include_is_skipped() {
    let tmp = TempDir::new().unwrap();
    write_file(
        tmp.path(),
        "guarded.h",
        "#ifndef GUARDED_H\n#define GUARDED_H\nint guarded_marker;\n#endif\n",
    );
    let main_path = write_file(
        tmp.path(),
        "main.c",
        "#include \"guarded.h\"\n#include \"guarded.h\"\n",
    );

    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main_path).unwrap();
    let diags = pp.take_diagnostics();
    assert!(diags.is_empty_or_no_errors(), "diags: {diags:?}");
    let marker_count = identifier_names(&out)
        .iter()
        .filter(|s| *s == "guarded_marker")
        .count();
    assert_eq!(marker_count, 1, "include guard should fire only once");
}

// Test F — an `#include` chain that loops back onto itself must be
// refused with a diagnostic (otherwise the preprocessor would recurse
// forever).
#[test]
fn circular_include_is_detected_and_reported() {
    let tmp = TempDir::new().unwrap();
    // a.h -> b.h -> a.h  (no guards)
    write_file(tmp.path(), "a.h", "#include \"b.h\"\n");
    write_file(tmp.path(), "b.h", "#include \"a.h\"\n");
    let main_path = write_file(tmp.path(), "main.c", "#include \"a.h\"\n");

    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let _out = pp.run_file(&main_path).unwrap();
    let diags = pp.take_diagnostics();
    assert!(
        diags
            .iter()
            .any(|d| matches!(d.severity, Severity::Error) && d.message.contains("circular")),
        "expected a circular-include error, got {diags:?}"
    );
}

// Test G — a missing header is a hard error.
#[test]
fn missing_header_produces_cannot_find_error() {
    let tmp = TempDir::new().unwrap();
    let main_path = write_file(tmp.path(), "main.c", "#include \"no_such_header.h\"\n");

    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let _ = pp.run_file(&main_path).unwrap();
    let diags = pp.take_diagnostics();
    assert!(
        diags
            .iter()
            .any(|d| matches!(d.severity, Severity::Error) && d.message.contains("cannot find")),
        "expected a cannot-find error, got {diags:?}"
    );
}

// Test L — with host-detected include paths, the core system headers
// preprocess without raising an error.  Skipped gracefully when the
// host has no usable toolchain.
#[test]
fn host_system_headers_preprocess_without_errors() {
    let paths = detect_system_include_paths();
    let have_stdio = paths
        .iter()
        .any(|p| p.join("stdio.h").is_file() || p.join("sys/cdefs.h").is_file());
    if !have_stdio {
        eprintln!("skipping system-header smoke test: no usable toolchain detected");
        return;
    }

    for header in ["stddef.h", "stdint.h", "limits.h", "stdio.h"] {
        let source = format!("#include <{header}>\n");
        let cfg = PreprocessConfig {
            include_paths: paths.clone(),
            ..PreprocessConfig::default()
        };
        let mut pp = Preprocessor::new(cfg);
        let _ = pp.run(lex(&source));
        let diags = pp.take_diagnostics();
        assert!(
            !diags.iter().any(|d| matches!(d.severity, Severity::Error)),
            "<{header}> produced errors: {diags:?}"
        );
    }
}

// Test M — a computed include whose header name comes from a macro.
#[test]
fn computed_include_expands_before_it_is_resolved() {
    let tmp = TempDir::new().unwrap();
    write_file(tmp.path(), "picked.h", "int picked_marker;\n");
    let main_path = write_file(
        tmp.path(),
        "main.c",
        "#define HDR \"picked.h\"\n#include HDR\n",
    );

    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main_path).unwrap();
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let names = identifier_names(&out);
    assert!(names.contains(&"picked_marker".to_string()));
    let _ = non_eof(&out);
}

#[test]
fn pragma_once_still_short_circuits_reinclusion() {
    // Regression guard for the rewrite: `#pragma once` must still
    // cause a second `#include` of the same file to be skipped.
    let tmp = TempDir::new().unwrap();
    write_file(tmp.path(), "once.h", "#pragma once\nint once_marker;\n");
    let main_path = write_file(
        tmp.path(),
        "main.c",
        "#include \"once.h\"\n#include \"once.h\"\n",
    );
    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main_path).unwrap();
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let count = out
        .iter()
        .filter(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "once_marker"))
        .count();
    assert_eq!(count, 1, "second include should have been skipped");
}

#[test]
fn angle_include_does_not_search_current_file_directory() {
    // The quote form resolves relative to the including file; the
    // angle form must not, so a header that exists only beside the
    // source but not on the system path is unreachable from `<>`.
    let tmp = TempDir::new().unwrap();
    write_file(tmp.path(), "local_only.h", "int local_marker;\n");
    let main_path = write_file(tmp.path(), "main.c", "#include <local_only.h>\n");
    let cfg = PreprocessConfig {
        // Deliberately empty: simulates a system path that has no
        // `local_only.h` available.
        include_paths: Vec::new(),
        ..PreprocessConfig::default()
    };
    let mut pp = Preprocessor::new(cfg);
    let _ = pp.run_file(&main_path).unwrap();
    let diags = pp.take_diagnostics();
    assert!(
        diags.iter().any(|d| matches!(d.severity, Severity::Error)
            && d.message.contains("cannot find")
            && d.message.contains("system")),
        "angle include with no system path should fail: {diags:?}"
    );
}

#[test]
fn angle_include_resolves_through_configured_system_path() {
    // The sister case: once the directory is registered via
    // `include_paths`, `<local_only.h>` is found there.
    let tmp = TempDir::new().unwrap();
    write_file(tmp.path(), "sys_hdr.h", "int sys_marker;\n");
    let src_dir = TempDir::new().unwrap();
    let main_path = write_file(src_dir.path(), "main.c", "#include <sys_hdr.h>\n");
    let cfg = PreprocessConfig {
        include_paths: vec![tmp.path().to_path_buf()],
        ..PreprocessConfig::default()
    };
    let mut pp = Preprocessor::new(cfg);
    let out = pp.run_file(&main_path).unwrap();
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    assert!(identifier_names(&out).contains(&"sys_marker".to_string()));
}

#[test]
fn include_paths_are_searched_in_configured_order() {
    // When two directories each provide a `probe.h`, the one that
    // appears earlier in `include_paths` must win.
    let first_dir = TempDir::new().unwrap();
    write_file(first_dir.path(), "probe.h", "int first_marker;\n");
    let second_dir = TempDir::new().unwrap();
    write_file(second_dir.path(), "probe.h", "int second_marker;\n");
    let src_dir = TempDir::new().unwrap();
    let main_path = write_file(src_dir.path(), "main.c", "#include <probe.h>\n");
    let cfg = PreprocessConfig {
        include_paths: vec![
            first_dir.path().to_path_buf(),
            second_dir.path().to_path_buf(),
        ],
        ..PreprocessConfig::default()
    };
    let mut pp = Preprocessor::new(cfg);
    let out = pp.run_file(&main_path).unwrap();
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let names = identifier_names(&out);
    assert!(names.contains(&"first_marker".to_string()));
    assert!(!names.contains(&"second_marker".to_string()));
}

#[test]
fn include_depth_limit_rejects_overlong_chains() {
    // Build a chain a.h → b.h → c.h with a tiny depth budget so the
    // third include is the one that trips the limit.
    let tmp = TempDir::new().unwrap();
    write_file(tmp.path(), "c.h", "int deepest;\n");
    write_file(tmp.path(), "b.h", "#include \"c.h\"\n");
    write_file(tmp.path(), "a.h", "#include \"b.h\"\n");
    let main_path = write_file(tmp.path(), "main.c", "#include \"a.h\"\n");

    let cfg = PreprocessConfig {
        max_include_depth: 2,
        ..PreprocessConfig::default()
    };
    let mut pp = Preprocessor::new(cfg);
    let _ = pp.run_file(&main_path).unwrap();
    let diags = pp.take_diagnostics();
    assert!(
        diags.iter().any(
            |d| matches!(d.severity, Severity::Error) && d.message.contains("nesting too deep")
        ),
        "expected a depth-limit error: {diags:?}"
    );
}
