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
    clippy::float_cmp
)]

//! C17 lexer for the Forge compiler.
//!
//! The lexer converts a raw `&str` of C source into a stream of [`Token`]s.
//! Each token carries a byte-offset [`Span`] (for diagnostics) and two
//! preprocessor-oriented flags: `at_start_of_line` and `has_leading_space`.
//!
//! # Example
//!
//! ```
//! use forge_lexer::{Lexer, TokenKind};
//!
//! let mut lex = Lexer::new("int x");
//! let tokens = lex.tokenize();
//!
//! assert_eq!(tokens[0].kind, TokenKind::Int);
//! assert_eq!(tokens[1].kind, TokenKind::Identifier("x".to_string()));
//! assert_eq!(tokens[2].kind, TokenKind::Eof);
//! ```
//!
//! # Scope of the current phase
//!
//! Phases 1.1, 1.2, and 1.3 are all complete:
//!
//! * **1.1** — whitespace / comments, every punctuator, identifiers,
//!   and the full C17 keyword set.
//! * **1.2** — every C17 numeric literal: decimal / octal / hex integers
//!   with all suffix combinations (`u`, `l`, `ll`, `ul`, `lu`, `ull`,
//!   `llu`, etc., case-insensitive), decimal and hex floating-point
//!   constants (including `.5`, `1.`, `1e5`, `0x1.8p1`) with `f`/`l`
//!   suffixes.  Diagnostics are emitted for invalid octal digits,
//!   overflowed integer literals, hex floats missing their binary
//!   exponent, empty hex integers, and exponents without digits.
//! * **1.3** — every C17 character and string literal (all prefixes —
//!   `L`, `u`, `U`, `u8` — every escape sequence, octal and hex
//!   escapes, universal character names, and recovery from unterminated
//!   or malformed literals).
//!
//! All diagnostics accumulated during lexing can be retrieved via
//! [`Lexer::take_diagnostics`].

use std::fmt;

pub mod lexer;
pub mod literal;
pub mod numeric;
pub mod token;

#[cfg(test)]
mod validation;

pub use forge_diagnostics::Diagnostic;
pub use lexer::{lex_fragment, lookup_keyword, Lexer};
pub use token::{CharPrefix, FloatSuffix, IntSuffix, StringPrefix, Token, TokenKind};

/// A byte-offset range within the source text.
///
/// Stored as `u32` on both ends so a [`Token`] stays small; this limits
/// translation-unit size to 4 GiB, which is well beyond any real C file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    /// Inclusive start byte offset.
    pub start: u32,
    /// Exclusive end byte offset.
    pub end: u32,
}

impl Span {
    /// Build a span from explicit start and end byte offsets.
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// The length of the span in bytes.
    pub const fn len(&self) -> u32 {
        self.end - self.start
    }

    /// Whether the span covers zero bytes.
    pub const fn is_empty(&self) -> bool {
        self.end == self.start
    }

    /// Convert to a [`std::ops::Range<usize>`] for use with byte-indexing APIs
    /// and [`forge_diagnostics`](https://docs.rs) spans.
    pub fn range(&self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

impl fmt::Display for Span {
    /// Renders the span as `"start..end"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_display() {
        assert_eq!(Span::new(0, 0).to_string(), "0..0");
        assert_eq!(Span::new(5, 10).to_string(), "5..10");
        assert_eq!(Span::new(123, 456).to_string(), "123..456");
    }

    #[test]
    fn span_len_and_is_empty() {
        assert_eq!(Span::new(0, 0).len(), 0);
        assert!(Span::new(0, 0).is_empty());
        assert_eq!(Span::new(3, 7).len(), 4);
        assert!(!Span::new(3, 7).is_empty());
    }

    #[test]
    fn span_range() {
        let s = Span::new(5, 10);
        assert_eq!(s.range(), 5_usize..10_usize);
    }
}
