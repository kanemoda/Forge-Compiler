// Pedantic lints we've audited and accept as style preferences for this crate.
#![allow(
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::unreadable_literal,
    clippy::doc_markdown,
    clippy::wildcard_imports,
    clippy::needless_pass_by_value,
    clippy::manual_let_else,
    clippy::match_wildcard_for_single_variants,
    clippy::redundant_closure_for_method_calls,
    clippy::float_cmp,
    clippy::default_trait_access,
    clippy::needless_raw_string_hashes,
    // `match` shapes that collapse to `if let` are clearer as-is when they
    // carry non-trivial error paths.
    clippy::single_match_else,
    // Diagnostic paths where the "negative" branch is the common case read
    // more naturally with `if !cond { handle_error }`.
    clippy::if_not_else,
    // Short enum names (`BinOp::*`) inside narrow helpers stay readable.
    clippy::enum_glob_use,
    // Preserved as plain `if` when it mirrors a truth-table in the spec.
    clippy::bool_to_int_with_if,
    // Module-local const arrays placed near their point of use.
    clippy::items_after_statements,
    clippy::map_unwrap_or,
    clippy::single_char_pattern
)]

//! C17 preprocessor for the Forge compiler.
//!
//! The preprocessor consumes a `Vec<Token>` produced by [`forge_lexer`]
//! and produces a new `Vec<Token>` with every preprocessing directive
//! consumed, every conditional block resolved, and (once the expansion
//! engine lands) every macro fully expanded.  The parser never sees a
//! `#` directive token.
//!
//! # Overview
//!
//! ```text
//! Vec<Token>  ──►  ┌────────────────────┐  ──►  Vec<Token>
//!   (lexer)        │  TokenCursor       │        (parser-ready)
//!                  │     │              │
//!                  │     ▼              │
//!                  │  directive         │
//!                  │    dispatch        │
//!                  │    │ ┌── #define   │
//!                  │    │ ├── #undef    │
//!                  │    │ ├── #if …     │
//!                  │    │ ├── #include  │
//!                  │    │ └── …         │
//!                  │    ▼                │
//!                  │  macro expansion    │
//!                  └────────────────────┘
//! ```
//!
//! # Current state (Prompt 2.1)
//!
//! This file covers the **skeleton** of Phase 2: the [`TokenCursor`]
//! (including `push_front` used for macro-expansion rescanning and
//! `#include` splicing), the [`Preprocessor`] main loop, and the
//! `#define` / `#undef` handlers — including the C17 §6.10.3/2
//! redefinition check that warns when two definitions for the same name
//! differ.
//!
//! All other directives (`#if`, `#ifdef`, `#elif`, `#else`, `#endif`,
//! `#include`, `#error`, `#warning`, `#line`, `#pragma`) are recognised
//! in the dispatch but their handlers are stubs that record an
//! informational note and consume their argument line without
//! interpreting it.  Macro **expansion** itself is not yet wired up —
//! every non-directive token currently passes through verbatim.
//!
//! # Example
//!
//! ```
//! use forge_diagnostics::FileId;
//! use forge_lexer::Lexer;
//! use forge_preprocess::{preprocess, PreprocessConfig, Preprocessor};
//!
//! let tokens = Lexer::new("#define N 42\nint x = N;", FileId::PRIMARY).tokenize();
//!
//! // High-level: short-circuit on error.
//! let _ = preprocess(tokens.clone(), PreprocessConfig::default());
//!
//! // Low-level: keeps warnings alongside the output stream.
//! let mut pp = Preprocessor::new(PreprocessConfig::default());
//! let _out = pp.run(tokens);
//! let _diags = pp.take_diagnostics();
//! assert!(pp.macros().contains_key("N"));
//! ```

pub mod cond_expr;
pub mod cursor;
pub mod expand;
pub mod pp_token;
pub mod preprocessor;
pub mod state;
pub mod system_includes;

pub use cond_expr::{evaluate as evaluate_cond_expression, PPValue};
pub use cursor::TokenCursor;
pub use expand::{paste_spelling, spelling_of, stringify};
pub use pp_token::{unwrap_tokens, wrap_tokens, PPToken};
pub use preprocessor::{preprocess, Preprocessor};
pub use state::{IfState, IncludeFrame, MacroDef, PreprocessConfig, TargetArch};
pub use system_includes::detect_system_include_paths;

// Re-export diagnostic/token types downstream crates will almost always
// want alongside the preprocessor API.
pub use forge_diagnostics::{Diagnostic, FileId, Severity, SourceMap};
pub use forge_lexer::{Token, TokenKind};

#[cfg(test)]
mod tests;
