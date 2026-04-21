//! Character and string literal sub-lexer.
//!
//! The entry points are [`Lexer::lex_char_literal`] and
//! [`Lexer::lex_string_literal`].  Each assumes its caller has already
//! consumed any prefix (`L`, `u`, `u8`, `U`) and that the cursor points
//! at the opening quote.  The sub-lexer consumes the quote, the body
//! (including every escape sequence), and — if present — the closing
//! quote, returning the appropriate [`TokenKind`].
//!
//! # Error recovery
//!
//! Malformed literals never cause the lexer to bail; instead a
//! [`Diagnostic`] is recorded on the lexer and scanning continues.  The
//! concrete recovery rules are:
//!
//! * **Unterminated literal** — scanning stops at the first unescaped
//!   newline or at EOF.  The newline itself is *not* consumed, so it is
//!   still available to terminate the enclosing line.  A token carrying
//!   whatever body was scanned is emitted.
//! * **Empty character constant** (`''`) — emits a diagnostic; the token
//!   still has `value == 0`.
//! * **Unknown escape** (`\q`) — emits a warning, consumes the escape
//!   character, and falls back to the raw value of that character.
//! * **`\x` with no hex digits** — emits an error; the escape contributes
//!   nothing to the literal.
//! * **Incomplete UCN** (`\u1`, `\U1234`) — emits an error; the escape
//!   contributes nothing to the literal.
//!
//! # Value model
//!
//! * [`CharLiteral::value`](crate::token::TokenKind::CharLiteral) stores a
//!   single `u32` code point.  For a multi-character narrow constant such
//!   as `'ab'` the value is `('a' << 8) | 'b' == 0x6162` (GCC's
//!   implementation-defined behaviour).
//! * [`StringLiteral::value`](crate::token::TokenKind::StringLiteral)
//!   stores a Rust `String`, i.e., UTF-8 bytes.  Every escape (narrow or
//!   wide) is interpreted as a Unicode scalar and appended in UTF-8 form;
//!   downstream code can re-encode to the target string encoding using
//!   the literal's [`StringPrefix`](crate::token::StringPrefix).

use forge_diagnostics::{Diagnostic, Span};

use crate::lexer::Lexer;
use crate::token::{CharPrefix, StringPrefix, TokenKind};

/// The outcome of consuming a single `\`-escape.
///
/// Separated from a plain `Option<u32>` so the sub-lexer can distinguish
/// "this escape produced no code point but recovered cleanly" (for
/// instance an `\x` with zero hex digits, where the diagnostic has
/// already been emitted) from "this escape was a line continuation and
/// must not contribute anything to the literal".
enum EscapeResult {
    /// The escape produced a single code point.
    Value(u32),
    /// A backslash at end-of-line — the caller must treat this as
    /// nothing at all (translation-phase-2 line continuation).  Only
    /// valid inside string literals; see [`Lexer::lex_string_literal`].
    LineContinuation,
    /// The escape was malformed; a diagnostic has already been emitted
    /// and the cursor has been advanced past the malformed portion.  The
    /// caller should not append anything to the literal's value.
    Error,
}

impl Lexer<'_> {
    // ---------------------------------------------------------------------
    // Public entry points (package-private — driven from `lexer.rs`).
    // ---------------------------------------------------------------------

    /// Lex a character literal.
    ///
    /// Precondition: `self.pos` points at the opening `'`.  The caller
    /// has already consumed any prefix (`L`, `u`, `U`) and passed the
    /// corresponding [`CharPrefix`].
    pub(crate) fn lex_char_literal(&mut self, prefix: CharPrefix) -> TokenKind {
        debug_assert_eq!(self.peek(), Some(b'\''));
        let open_pos = self.pos;
        self.pos += 1; // consume opening quote

        let mut values: Vec<u32> = Vec::new();
        let mut closed = false;

        loop {
            match self.peek() {
                // EOF, or a raw newline — both terminate the literal without
                // being consumed.  C forbids unescaped newlines in literals.
                None | Some(b'\n' | b'\r') => break,
                Some(b'\'') => {
                    self.pos += 1;
                    closed = true;
                    break;
                }
                Some(b'\\') => match self.consume_escape_sequence() {
                    EscapeResult::Value(v) => values.push(v),
                    // Line continuation inside a character constant is
                    // rare but legal (translation phase 2); accept it.
                    EscapeResult::LineContinuation | EscapeResult::Error => {}
                },
                Some(_) => {
                    let ch = self.consume_unicode_char();
                    values.push(ch as u32);
                }
            }
        }

        if !closed {
            self.emit_diagnostic(
                Diagnostic::error("unterminated character constant")
                    .span(Span::new(self.file_id, open_pos as u32, self.pos as u32))
                    .label("unterminated character constant starts here")
                    .note("C character constants must be closed with `'` on the same line"),
            );
        } else if values.is_empty() {
            self.emit_diagnostic(
                Diagnostic::error("empty character constant")
                    .span(Span::new(self.file_id, open_pos as u32, self.pos as u32))
                    .label("a character constant must contain at least one character"),
            );
        }

        let value = combine_char_values(self, prefix, &values, open_pos);
        TokenKind::CharLiteral { value, prefix }
    }

    /// Lex a string literal.
    ///
    /// Precondition: `self.pos` points at the opening `"`.  Any prefix
    /// (`L`, `u8`, `u`, `U`) has already been consumed by the caller.
    pub(crate) fn lex_string_literal(&mut self, prefix: StringPrefix) -> TokenKind {
        debug_assert_eq!(self.peek(), Some(b'"'));
        let open_pos = self.pos;
        self.pos += 1; // consume opening quote

        let mut value = String::new();
        let mut closed = false;

        loop {
            match self.peek() {
                // EOF or a raw newline — both terminate the literal.
                None | Some(b'\n' | b'\r') => break,
                Some(b'"') => {
                    self.pos += 1;
                    closed = true;
                    break;
                }
                Some(b'\\') => match self.consume_escape_sequence() {
                    EscapeResult::Value(v) => push_code_point(&mut value, v),
                    EscapeResult::LineContinuation | EscapeResult::Error => {}
                },
                Some(_) => {
                    let ch = self.consume_unicode_char();
                    value.push(ch);
                }
            }
        }

        if !closed {
            self.emit_diagnostic(
                Diagnostic::error("unterminated string literal")
                    .span(Span::new(self.file_id, open_pos as u32, self.pos as u32))
                    .label("unterminated string literal starts here")
                    .note("C string literals must be closed with `\"` on the same line"),
            );
        }

        TokenKind::StringLiteral { value, prefix }
    }

    // ---------------------------------------------------------------------
    // Escape sequence machinery.
    // ---------------------------------------------------------------------

    /// Consume a single `\`-escape starting at the current cursor.
    ///
    /// Advances the cursor past the entire escape (including any
    /// digits); on success returns its code-point value.  Error
    /// recovery: malformed escapes emit a diagnostic and return
    /// [`EscapeResult::Error`] without re-raising.
    fn consume_escape_sequence(&mut self) -> EscapeResult {
        debug_assert_eq!(self.peek(), Some(b'\\'));
        let esc_start = self.pos;
        self.pos += 1; // consume `\`

        let Some(c) = self.peek() else {
            self.emit_diagnostic(
                Diagnostic::error("incomplete escape sequence at end of input")
                    .span(Span::new(self.file_id, esc_start as u32, self.pos as u32))
                    .label("a `\\` must be followed by an escape character"),
            );
            return EscapeResult::Error;
        };

        match c {
            // Line continuation — `\` immediately followed by a newline
            // is translation-phase-2 and must produce no value.
            b'\n' => {
                self.pos += 1;
                EscapeResult::LineContinuation
            }
            b'\r' => {
                self.pos += 1;
                if self.peek() == Some(b'\n') {
                    self.pos += 1;
                }
                EscapeResult::LineContinuation
            }

            // Single-character simple escapes (C17 6.4.4.4).
            b'a' => self.single_escape(0x07),
            b'b' => self.single_escape(0x08),
            b'f' => self.single_escape(0x0C),
            b'n' => self.single_escape(0x0A),
            b'r' => self.single_escape(0x0D),
            b't' => self.single_escape(0x09),
            b'v' => self.single_escape(0x0B),
            b'\\' => self.single_escape(0x5C),
            b'\'' => self.single_escape(0x27),
            b'"' => self.single_escape(0x22),
            b'?' => self.single_escape(0x3F),

            // Octal escape: 1 to 3 octal digits.
            b'0'..=b'7' => EscapeResult::Value(self.consume_octal_escape()),

            // Hex escape: `\x` followed by one or more hex digits.
            b'x' => self.consume_hex_escape(esc_start),

            // Universal character names.
            b'u' => self.consume_universal_char(esc_start, 4),
            b'U' => self.consume_universal_char(esc_start, 8),

            // Anything else — emit a warning but carry on with the raw
            // value so that subsequent parsing still makes progress.
            _ => {
                let bad_ch = self.consume_unicode_char();
                self.emit_diagnostic(
                    Diagnostic::warning(format!("unknown escape sequence: '\\{bad_ch}'"))
                        .span(Span::new(self.file_id, esc_start as u32, self.pos as u32))
                        .label("unknown escape sequence")
                        .note("valid escapes are \\a \\b \\f \\n \\r \\t \\v \\\\ \\' \\\" \\? \\0 \\xHH \\uHHHH \\UHHHHHHHH"),
                );
                EscapeResult::Value(bad_ch as u32)
            }
        }
    }

    /// Helper for single-character simple escapes (e.g., `\n` → `0x0A`).
    ///
    /// Advances past the character after the `\` and returns its
    /// canonical code-point value.
    fn single_escape(&mut self, value: u32) -> EscapeResult {
        self.pos += 1;
        EscapeResult::Value(value)
    }

    /// Consume a 1- to 3-digit octal escape, starting at the first
    /// octal digit (the `\` has already been consumed).
    fn consume_octal_escape(&mut self) -> u32 {
        let mut value: u32 = 0;
        for _ in 0..3 {
            match self.peek() {
                Some(c @ b'0'..=b'7') => {
                    value = value * 8 + (c - b'0') as u32;
                    self.pos += 1;
                }
                _ => break,
            }
        }
        value
    }

    /// Consume a `\x` hex escape — greedy over hex digits.  Requires at
    /// least one digit; emits an error if none follow.
    fn consume_hex_escape(&mut self, esc_start: usize) -> EscapeResult {
        self.pos += 1; // consume 'x'

        let mut value: u32 = 0;
        let mut digits = 0u32;
        let mut overflowed = false;

        while let Some(c) = self.peek() {
            let digit = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => break,
            };
            if value > (u32::MAX >> 4) {
                overflowed = true;
            }
            value = value.wrapping_mul(16).wrapping_add(digit as u32);
            self.pos += 1;
            digits += 1;
        }

        if digits == 0 {
            self.emit_diagnostic(
                Diagnostic::error("\\x used with no following hex digits")
                    .span(Span::new(self.file_id, esc_start as u32, self.pos as u32))
                    .label("expected one or more hex digits after `\\x`"),
            );
            return EscapeResult::Error;
        }

        if overflowed {
            self.emit_diagnostic(
                Diagnostic::warning("hex escape sequence overflows 32 bits")
                    .span(Span::new(self.file_id, esc_start as u32, self.pos as u32))
                    .label("value truncated to the low 32 bits"),
            );
        }

        EscapeResult::Value(value)
    }

    /// Consume a Universal Character Name: `\uHHHH` (n = 4) or
    /// `\UHHHHHHHH` (n = 8).  Exactly `n` hex digits are required.
    fn consume_universal_char(&mut self, esc_start: usize, n: usize) -> EscapeResult {
        self.pos += 1; // consume 'u' or 'U'

        let mut value: u32 = 0;
        for i in 0..n {
            let Some(c) = self.peek() else {
                self.emit_diagnostic(
                    Diagnostic::error(format!(
                        "incomplete universal character name: expected {n} hex digits, found {i}"
                    ))
                    .span(Span::new(self.file_id, esc_start as u32, self.pos as u32))
                    .label("universal character name truncated here"),
                );
                return EscapeResult::Error;
            };
            let digit = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => {
                    self.emit_diagnostic(
                        Diagnostic::error(format!(
                            "incomplete universal character name: expected {n} hex digits, found {i}"
                        ))
                        .span(Span::new(self.file_id, esc_start as u32, self.pos as u32))
                        .label("non-hex digit in universal character name"),
                    );
                    return EscapeResult::Error;
                }
            };
            value = (value << 4) | digit as u32;
            self.pos += 1;
        }

        EscapeResult::Value(value)
    }
}

// -------------------------------------------------------------------------
// Free helpers.
// -------------------------------------------------------------------------

/// Append a code point to a string-literal body, substituting U+FFFD
/// for any value that is not a valid Unicode scalar.
///
/// Diagnostics for out-of-range escapes (surrogates, > 0x10FFFF) are
/// emitted by the caller at the site of the escape, so this helper
/// silently repairs the result so the stored `String` stays valid UTF-8.
fn push_code_point(out: &mut String, value: u32) {
    match char::from_u32(value) {
        Some(ch) => out.push(ch),
        None => out.push('\u{FFFD}'),
    }
}

/// Compose a character-literal's final `u32` value from the list of
/// decoded elements, handling the multi-character (narrow) and wide
/// (`L` / `u` / `U`) cases separately.
///
/// Narrow multi-character constants (`'ab'`) match GCC's behaviour:
/// successive bytes are packed into the result by shifting the running
/// total left 8 bits and `OR`-ing in the next byte.  Wide prefixes allow
/// only a single character; anything more generates a warning and the
/// first value is returned.
fn combine_char_values(
    lexer: &mut Lexer<'_>,
    prefix: CharPrefix,
    values: &[u32],
    open_pos: usize,
) -> u32 {
    match values.len() {
        0 => 0,
        1 => values[0],
        _ => {
            if prefix == CharPrefix::None {
                // Pack bytes left-to-right, per GCC's implementation-
                // defined behaviour for multi-character constants.
                let mut result: u32 = 0;
                for v in values {
                    result = result.wrapping_shl(8) | (*v & 0xFF);
                }
                result
            } else {
                lexer.emit_diagnostic(
                    Diagnostic::warning("multi-character wide character constant")
                        .span(Span::new(lexer.file_id, open_pos as u32, lexer.pos as u32))
                        .label("only the first character is retained")
                        .note("wide character constants (L'...', u'...', U'...') may contain a single character"),
                );
                values[0]
            }
        }
    }
}
