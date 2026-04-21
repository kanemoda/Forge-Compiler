//! Preprocessor state types.
//!
//! This module defines the small, mostly-data types the preprocessor
//! manipulates as it walks a translation unit:
//!
//! * [`MacroDef`] — a stored macro definition (object- or function-like).
//! * [`IfState`]  — the per-conditional-block state kept on the `if`-stack.
//! * [`IncludeFrame`] — a single frame on the include stack, used for
//!   depth limits and circular-include detection.
//! * [`PreprocessConfig`] — the caller-supplied configuration (include
//!   paths, target architecture, command-line `-D` definitions).
//! * [`TargetArch`] — the architecture the preprocessor is configured for,
//!   which drives predefined macros such as `__x86_64__` /
//!   `__aarch64__`.

use std::path::PathBuf;

use forge_lexer::{Span, Token};

// ---------------------------------------------------------------------------
// Macro definitions
// ---------------------------------------------------------------------------

/// A single stored macro definition.
///
/// Both variants keep the macro's name inline (rather than only as a map
/// key) so a [`MacroDef`] is self-describing when it is inspected in a
/// diagnostic or a debug dump.
#[derive(Clone, Debug, PartialEq)]
pub enum MacroDef {
    /// An object-like macro: `#define NAME replacement...`.
    ObjectLike {
        /// The macro name.
        name: String,
        /// The replacement list, verbatim, as lexed from the directive
        /// line.  Whitespace between replacement tokens is carried on each
        /// token's [`Token::has_leading_space`] flag.
        replacement: Vec<Token>,
        /// `true` if this macro is one the compiler itself synthesises —
        /// `__FILE__`, `__LINE__`, `__DATE__`, etc.  Predefined macros
        /// get special handling during expansion (e.g. `__LINE__` must
        /// expand to the current line, not the line it was defined on).
        is_predefined: bool,
    },
    /// A function-like macro: `#define NAME(params) replacement...`.
    FunctionLike {
        /// The macro name.
        name: String,
        /// The named parameters, in declaration order.  For a variadic
        /// macro the trailing `...` is **not** stored here — see
        /// [`is_variadic`](Self::FunctionLike::is_variadic).
        params: Vec<String>,
        /// `true` if the parameter list ended with `...`; the extra
        /// arguments are exposed inside the replacement list as
        /// `__VA_ARGS__`.
        is_variadic: bool,
        /// The replacement list.  See [`ObjectLike::replacement`].
        replacement: Vec<Token>,
    },
}

impl MacroDef {
    /// The macro's name, regardless of its kind.
    pub fn name(&self) -> &str {
        match self {
            MacroDef::ObjectLike { name, .. } | MacroDef::FunctionLike { name, .. } => name,
        }
    }

    /// The replacement token list for either flavour.
    pub fn replacement(&self) -> &[Token] {
        match self {
            MacroDef::ObjectLike { replacement, .. }
            | MacroDef::FunctionLike { replacement, .. } => replacement,
        }
    }

    /// `true` if `self` is a function-like macro.
    pub fn is_function_like(&self) -> bool {
        matches!(self, MacroDef::FunctionLike { .. })
    }

    /// `true` if `self` is a variadic function-like macro
    /// (`#define F(x, ...) ...`).  Always `false` for object-like macros.
    pub fn is_variadic(&self) -> bool {
        matches!(
            self,
            MacroDef::FunctionLike {
                is_variadic: true,
                ..
            }
        )
    }
}

/// Are two definitions equivalent for the purposes of C17 §6.10.3/2?
///
/// The standard requires that a redefinition be accepted **without a
/// diagnostic** iff the two replacement lists have the same number,
/// ordering, spelling, and whitespace separation — but every run of
/// whitespace counts as identical.  For function-like macros the parameter
/// lists must also match.
///
/// The `is_predefined` flag is ignored: it is a compiler-internal marker,
/// not part of the user-visible definition.
pub fn macros_equivalent(a: &MacroDef, b: &MacroDef) -> bool {
    match (a, b) {
        (
            MacroDef::ObjectLike {
                replacement: ra, ..
            },
            MacroDef::ObjectLike {
                replacement: rb, ..
            },
        ) => replacements_equivalent(ra, rb),
        (
            MacroDef::FunctionLike {
                params: pa,
                is_variadic: va,
                replacement: ra,
                ..
            },
            MacroDef::FunctionLike {
                params: pb,
                is_variadic: vb,
                replacement: rb,
                ..
            },
        ) => pa == pb && va == vb && replacements_equivalent(ra, rb),
        _ => false,
    }
}

/// Compare two replacement token lists modulo whitespace amount.
fn replacements_equivalent(a: &[Token], b: &[Token]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| x.kind == y.kind && x.has_leading_space == y.has_leading_space)
}

// ---------------------------------------------------------------------------
// Conditional compilation state
// ---------------------------------------------------------------------------

/// One entry on the preprocessor's `#if`/`#ifdef` stack.
///
/// Conditional compilation is implemented by pushing an [`IfState`] when
/// an `#if`/`#ifdef`/`#ifndef` is seen, mutating it on `#elif` / `#else`,
/// and popping it on `#endif`.  A block is actively emitting tokens iff
/// every frame on the stack has `current_branch_active == true`.
#[derive(Clone, Debug)]
pub struct IfState {
    /// `true` once any branch of this `#if`/`#elif` chain has evaluated to
    /// true.  Once a branch is taken, every subsequent `#elif`/`#else`
    /// branch is inactive even if its own condition evaluates to true.
    pub any_branch_taken: bool,
    /// `true` iff this frame's current branch is active.  When any
    /// enclosing frame is inactive, this is forced to `false`.
    pub current_branch_active: bool,
    /// `true` once a `#else` has been seen for this frame — a second one
    /// (or a following `#elif`) is a diagnostic.
    pub else_seen: bool,
    /// Source location of the opening `#if` / `#ifdef` / `#ifndef`, used
    /// for the "unterminated conditional" error at end-of-file.
    pub if_location: Span,
}

impl IfState {
    /// Build an [`IfState`] for a newly-opened `#if` that starts out with
    /// its first branch taken iff `active` is `true`.
    pub fn new(active: bool, if_location: Span) -> Self {
        Self {
            any_branch_taken: active,
            current_branch_active: active,
            else_seen: false,
            if_location,
        }
    }
}

// ---------------------------------------------------------------------------
// Include stack
// ---------------------------------------------------------------------------

/// One entry on the include stack — records a file that is currently being
/// preprocessed.
///
/// Used for (a) detecting circular `#include` chains, (b) enforcing a
/// maximum include depth, and (c) resolving `#include "..."` relative to
/// the directory of the current file.
#[derive(Clone, Debug)]
pub struct IncludeFrame {
    /// The display filename — what `__FILE__` expands to while this frame
    /// is current.  For the top-level translation unit this is typically
    /// `"<input>"` or the user-supplied filename; for an `#include`d file
    /// it is the canonical path as a string.
    pub filename: String,
    /// Canonical filesystem path of the file, when available.  `None` for
    /// the top-level unit when it was handed to the preprocessor as a
    /// bare [`Vec<Token>`](forge_lexer::Token) with no on-disk source.
    /// Used for circular-include detection and for the quote-include
    /// search (the directory containing this path is searched first).
    pub path: Option<PathBuf>,
    /// Depth of this frame: the top-level translation unit is depth `0`,
    /// the file it `#include`s is depth `1`, and so on.
    pub depth: u32,
}

impl IncludeFrame {
    /// Create a new include frame with just a display filename (no
    /// canonical path).  Primarily for the top-level synthetic frame.
    pub fn new(filename: impl Into<String>, depth: u32) -> Self {
        Self {
            filename: filename.into(),
            path: None,
            depth,
        }
    }

    /// Create an include frame rooted at an on-disk file.  The display
    /// filename is derived from the path's string representation.
    pub fn file(path: PathBuf, depth: u32) -> Self {
        Self {
            filename: path.display().to_string(),
            path: Some(path),
            depth,
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Target architecture the preprocessor is configured for.
///
/// This determines which architecture-specific predefined macros (such as
/// `__x86_64__` or `__aarch64__`) are injected into the macro table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TargetArch {
    /// 64-bit x86 — `__x86_64__`, `__LP64__`, etc.
    #[default]
    X86_64,
    /// 64-bit ARM — `__aarch64__`, `__LP64__`, etc.
    AArch64,
}

/// Default `#include` nesting depth beyond which the preprocessor
/// aborts — generous enough to clear deeply layered system headers
/// (the usual suspects peak around 30–40 levels) while still catching
/// indirect cycles long before the process runs out of stack.
pub const DEFAULT_MAX_INCLUDE_DEPTH: u32 = 200;

/// Caller-supplied preprocessor configuration.
///
/// Sensible defaults: no include paths, [`TargetArch::X86_64`], no
/// command-line predefined macros, and
/// [`DEFAULT_MAX_INCLUDE_DEPTH`] nesting limit.
#[derive(Clone, Debug)]
pub struct PreprocessConfig {
    /// Directories searched for `#include <...>` headers, in order.
    /// Also searched (after the current file's directory) for
    /// `#include "..."`.  Populate via the `-I` command line option
    /// and/or [`crate::detect_system_include_paths`].
    pub include_paths: Vec<PathBuf>,
    /// Target architecture driving arch-specific predefined macros.
    pub target_arch: TargetArch,
    /// Additional user-supplied macros of the form `-DNAME=VALUE`.  Empty
    /// values (`-DNAME`) are defined as `1` per the conventional compiler
    /// CLI behaviour.
    pub predefined_macros: Vec<(String, String)>,
    /// Maximum `#include` nesting depth.  The preprocessor emits an
    /// error and aborts the offending include when this limit is
    /// exceeded.
    pub max_include_depth: u32,
}

impl Default for PreprocessConfig {
    fn default() -> Self {
        Self {
            include_paths: Vec::new(),
            target_arch: TargetArch::default(),
            predefined_macros: Vec::new(),
            max_include_depth: DEFAULT_MAX_INCLUDE_DEPTH,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use forge_diagnostics::FileId;
    use forge_lexer::Lexer;

    fn lex_body(src: &str) -> Vec<Token> {
        // Drop the trailing Eof so the replacement list is exactly the
        // source.
        let mut toks = Lexer::new(src, FileId::PRIMARY).tokenize();
        toks.pop();
        toks
    }

    #[test]
    fn object_like_accessors_report_the_name_and_replacement() {
        let m = MacroDef::ObjectLike {
            name: "FOO".into(),
            replacement: lex_body("42"),
            is_predefined: false,
        };
        assert_eq!(m.name(), "FOO");
        assert_eq!(m.replacement().len(), 1);
        assert!(!m.is_function_like());
        assert!(!m.is_variadic());
    }

    #[test]
    fn function_like_accessors_report_params_and_variadic() {
        let m = MacroDef::FunctionLike {
            name: "F".into(),
            params: vec!["x".into(), "y".into()],
            is_variadic: true,
            replacement: lex_body("x + y"),
        };
        assert_eq!(m.name(), "F");
        assert!(m.is_function_like());
        assert!(m.is_variadic());
    }

    #[test]
    fn equivalent_object_like_replacements_are_equivalent() {
        let a = MacroDef::ObjectLike {
            name: "X".into(),
            replacement: lex_body("1 + 2"),
            is_predefined: false,
        };
        let b = MacroDef::ObjectLike {
            name: "X".into(),
            replacement: lex_body("1 + 2"),
            is_predefined: false,
        };
        assert!(macros_equivalent(&a, &b));
    }

    #[test]
    fn object_like_replacements_ignore_whitespace_amount() {
        let a = MacroDef::ObjectLike {
            name: "X".into(),
            replacement: lex_body("1 + 2"),
            is_predefined: false,
        };
        let b = MacroDef::ObjectLike {
            name: "X".into(),
            replacement: lex_body("1    +    2"),
            is_predefined: false,
        };
        assert!(macros_equivalent(&a, &b));
    }

    #[test]
    fn different_object_like_replacements_are_not_equivalent() {
        let a = MacroDef::ObjectLike {
            name: "X".into(),
            replacement: lex_body("1"),
            is_predefined: false,
        };
        let b = MacroDef::ObjectLike {
            name: "X".into(),
            replacement: lex_body("2"),
            is_predefined: false,
        };
        assert!(!macros_equivalent(&a, &b));
    }

    #[test]
    fn object_and_function_like_are_never_equivalent() {
        let a = MacroDef::ObjectLike {
            name: "X".into(),
            replacement: lex_body("1"),
            is_predefined: false,
        };
        let b = MacroDef::FunctionLike {
            name: "X".into(),
            params: vec![],
            is_variadic: false,
            replacement: lex_body("1"),
        };
        assert!(!macros_equivalent(&a, &b));
    }

    #[test]
    fn function_like_parameter_lists_must_match_exactly() {
        let a = MacroDef::FunctionLike {
            name: "F".into(),
            params: vec!["x".into()],
            is_variadic: false,
            replacement: lex_body("x"),
        };
        let b = MacroDef::FunctionLike {
            name: "F".into(),
            params: vec!["y".into()],
            is_variadic: false,
            replacement: lex_body("x"),
        };
        assert!(!macros_equivalent(&a, &b));
    }

    #[test]
    fn variadic_mismatch_is_not_equivalent() {
        let a = MacroDef::FunctionLike {
            name: "F".into(),
            params: vec!["x".into()],
            is_variadic: false,
            replacement: lex_body("x"),
        };
        let b = MacroDef::FunctionLike {
            name: "F".into(),
            params: vec!["x".into()],
            is_variadic: true,
            replacement: lex_body("x"),
        };
        assert!(!macros_equivalent(&a, &b));
    }

    #[test]
    fn if_state_new_seeds_from_active_flag() {
        let on = IfState::new(true, Span::primary(0, 3));
        assert!(on.any_branch_taken);
        assert!(on.current_branch_active);
        assert!(!on.else_seen);

        let off = IfState::new(false, Span::primary(0, 3));
        assert!(!off.any_branch_taken);
        assert!(!off.current_branch_active);
        assert!(!off.else_seen);
    }

    #[test]
    fn preprocess_config_default_is_x86_64_no_paths() {
        let cfg = PreprocessConfig::default();
        assert!(cfg.include_paths.is_empty());
        assert_eq!(cfg.target_arch, TargetArch::X86_64);
        assert!(cfg.predefined_macros.is_empty());
        assert_eq!(cfg.max_include_depth, DEFAULT_MAX_INCLUDE_DEPTH);
    }

    #[test]
    fn include_frame_preserves_filename_and_depth() {
        let f = IncludeFrame::new("/usr/include/stdio.h", 2);
        assert_eq!(f.filename, "/usr/include/stdio.h");
        assert_eq!(f.depth, 2);
        assert!(f.path.is_none());
    }

    #[test]
    fn include_frame_file_keeps_canonical_path() {
        let f = IncludeFrame::file(PathBuf::from("/tmp/foo.h"), 1);
        assert_eq!(f.filename, "/tmp/foo.h");
        assert_eq!(f.path.as_deref(), Some(std::path::Path::new("/tmp/foo.h")));
        assert_eq!(f.depth, 1);
    }
}
