// Pedantic lints we've audited and accept as style preferences for this crate.
#![allow(
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::doc_markdown,
    clippy::wildcard_imports,
    clippy::needless_pass_by_value,
    clippy::manual_let_else,
    clippy::match_wildcard_for_single_variants,
    // `match` shapes that collapse to `if let` are clearer as-is when they
    // carry non-trivial error paths (extracting identifier + emitting
    // diagnostic on the error arm).
    clippy::single_match_else,
    // Stress-test source-text builders read more naturally as a series
    // of `push_str(&format!(..))` calls than as `write!` invocations
    // with a separate `std::fmt::Write` import.
    clippy::format_push_string
)]

//! C17 parser for the Forge compiler.
//!
//! This crate consumes a preprocessed token stream produced by
//! [`forge_preprocess`] and builds a complete C17 AST.  The AST types
//! are intentionally *syntactic* — they mirror what the source says,
//! not what it means.  Type resolution, implicit conversions, and scope
//! analysis are deferred to Phase 4 (semantic analysis).
//!
//! # Entry point
//!
//! ```ignore
//! use forge_parser::Parser;
//! let (tu, diagnostics) = Parser::parse(tokens);
//! ```
//!
//! The parser never panics on malformed input — every syntactic error
//! produces a diagnostic and the parser synchronises to the next
//! statement boundary so downstream tooling sees as much of the tree as
//! possible.
//!
//! [`forge_preprocess`]: https://docs.rs/forge_preprocess

pub mod ast;
pub mod ast_ops;
pub mod printer;

mod decl;
mod expr;
mod parser;
mod stmt;

#[cfg(test)]
mod tests;

pub use ast::TranslationUnit;
pub use parser::Parser;
