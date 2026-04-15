//! Runtime detection of the host system's C include search paths.
//!
//! The canonical trick for this is to ask the system's `cc` driver to
//! print its own search list with verbose preprocessing over an empty
//! input:
//!
//! ```text
//! cc -E -v -x c /dev/null
//! ```
//!
//! The driver writes the search paths to **stderr** between the markers
//! `#include <...> search starts here:` and `End of search list.`.
//! This module shells out, grabs that block, and returns the paths as
//! [`PathBuf`]s — or falls back to a pair of hard-coded locations if the
//! command can't be run at all.
//!
//! Detection is a *best effort*.  Missing `cc`, unparseable output, or
//! any I/O error all degrade to the fallback list: a misconfigured
//! environment still gets a usable preprocessor, it just can't find
//! system headers.

use std::path::PathBuf;
use std::process::Command;

/// Hard-coded fallback search paths used when `cc -E -v` cannot be run
/// (no compiler installed, sandboxed environment, etc.).
///
/// The choice is deliberately POSIX-flavoured: macOS, Linux, and BSD
/// all place the core headers under at least one of these directories
/// when a toolchain is present.
const FALLBACK_PATHS: &[&str] = &["/usr/include", "/usr/local/include"];

/// Detect the system's default `#include <...>` search paths by
/// shelling out to `cc -E -v -x c /dev/null`.
///
/// Returns the list of directories `cc` reports between
/// `#include <...> search starts here:` and `End of search list.`, in
/// the order `cc` gives them.  On any failure — the command cannot be
/// spawned, exits with a weird status, produces no markers, or yields
/// an empty list — returns [`FALLBACK_PATHS`] as [`PathBuf`]s so that
/// callers always get a non-empty search list.
///
/// macOS-style "(framework directory)" entries are filtered out since
/// they are not plain `-I` directories.
pub fn detect_system_include_paths() -> Vec<PathBuf> {
    match run_cc_verbose() {
        Some(paths) if !paths.is_empty() => paths,
        _ => fallback_paths(),
    }
}

/// Execute `cc -E -v -x c /dev/null` and extract its reported search
/// list.  Returns `None` on any spawn or I/O failure.
fn run_cc_verbose() -> Option<Vec<PathBuf>> {
    let output = Command::new("cc")
        .args(["-E", "-v", "-x", "c", "/dev/null"])
        .output()
        .ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    Some(parse_search_list(&stderr))
}

/// Parse the block between `#include <...> search starts here:` and
/// `End of search list.` out of `cc -v` stderr.
///
/// Exposed as a helper so the parse logic can be unit-tested without
/// actually invoking `cc`.
pub(crate) fn parse_search_list(stderr: &str) -> Vec<PathBuf> {
    let mut in_section = false;
    let mut paths: Vec<PathBuf> = Vec::new();
    for line in stderr.lines() {
        if !in_section {
            if line
                .trim_start()
                .starts_with("#include <...> search starts here:")
            {
                in_section = true;
            }
            continue;
        }
        if line.trim_start().starts_with("End of search list.") {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.ends_with("(framework directory)") {
            continue;
        }
        paths.push(PathBuf::from(trimmed));
    }
    paths
}

/// Promote [`FALLBACK_PATHS`] to a fresh [`Vec<PathBuf>`].
fn fallback_paths() -> Vec<PathBuf> {
    FALLBACK_PATHS.iter().map(PathBuf::from).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_search_list_extracts_indented_paths_between_markers() {
        let sample = "ignored leading line\n\
                      #include \"...\" search starts here:\n\
                      /quote/only\n\
                      #include <...> search starts here:\n\
                      /usr/local/include\n\
                      /usr/include\n\
                      End of search list.\n\
                      trailing junk\n";
        let paths = parse_search_list(sample);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/usr/local/include"),
                PathBuf::from("/usr/include"),
            ]
        );
    }

    #[test]
    fn parse_search_list_skips_framework_directories() {
        let sample = "#include <...> search starts here:\n\
                      /usr/include\n\
                      /System/Library/Frameworks (framework directory)\n\
                      End of search list.\n";
        let paths = parse_search_list(sample);
        assert_eq!(paths, vec![PathBuf::from("/usr/include")]);
    }

    #[test]
    fn parse_search_list_without_markers_returns_empty() {
        let sample = "cc: hello world\nno useful markers here\n";
        assert!(parse_search_list(sample).is_empty());
    }

    #[test]
    fn fallback_paths_is_never_empty() {
        assert!(!fallback_paths().is_empty());
    }
}
