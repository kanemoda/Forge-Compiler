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

use forge_diagnostics::Diagnostic;

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

impl<'a> Lexer<'a> {
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
                None => break,
                Some(b'\'') => {
                    self.pos += 1;
                    closed = true;
                    break;
                }
                // A raw newline terminates the literal without being
                // consumed — C forbids unescaped newlines in literals.
                Some(b'\n') | Some(b'\r') => break,
                Some(b'\\') => match self.consume_escape_sequence() {
                    EscapeResult::Value(v) => values.push(v),
                    // Line continuation inside a character constant is
                    // rare but legal (translation phase 2); accept it.
                    EscapeResult::LineContinuation => {}
                    EscapeResult::Error => {}
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
                    .span(open_pos..self.pos)
                    .label("unterminated character constant starts here")
                    .note("C character constants must be closed with `'` on the same line"),
            );
        } else if values.is_empty() {
            self.emit_diagnostic(
                Diagnostic::error("empty character constant")
                    .span(open_pos..self.pos)
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
                None => break,
                Some(b'"') => {
                    self.pos += 1;
                    closed = true;
                    break;
                }
                Some(b'\n') | Some(b'\r') => break,
                Some(b'\\') => match self.consume_escape_sequence() {
                    EscapeResult::Value(v) => push_code_point(&mut value, v),
                    EscapeResult::LineContinuation => {}
                    EscapeResult::Error => {}
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
                    .span(open_pos..self.pos)
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
                    .span(esc_start..self.pos)
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
                        .span(esc_start..self.pos)
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
                    .span(esc_start..self.pos)
                    .label("expected one or more hex digits after `\\x`"),
            );
            return EscapeResult::Error;
        }

        if overflowed {
            self.emit_diagnostic(
                Diagnostic::warning("hex escape sequence overflows 32 bits")
                    .span(esc_start..self.pos)
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
                    .span(esc_start..self.pos)
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
                        .span(esc_start..self.pos)
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
        _ => match prefix {
            CharPrefix::None => {
                // Pack bytes left-to-right, per GCC's implementation-
                // defined behaviour for multi-character constants.
                let mut result: u32 = 0;
                for v in values {
                    result = result.wrapping_shl(8) | (*v & 0xFF);
                }
                result
            }
            _ => {
                lexer.emit_diagnostic(
                    Diagnostic::warning("multi-character wide character constant")
                        .span(open_pos..lexer.pos)
                        .label("only the first character is retained")
                        .note("wide character constants (L'...', u'...', U'...') may contain a single character"),
                );
                values[0]
            }
        },
    }
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Lexer, Span, Token};

    // ---------- Test helpers ----------

    /// Tokenize and return the non-`Eof` tokens plus any diagnostics.
    fn lex_with_diags(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
        let mut lx = Lexer::new(src);
        let mut toks = lx.tokenize();
        let last = toks
            .pop()
            .expect("tokenize must always produce at least Eof");
        assert!(
            matches!(last.kind, TokenKind::Eof),
            "last token must be Eof"
        );
        let diags = lx.take_diagnostics();
        (toks, diags)
    }

    /// Tokenize and assert the result is a single token of the expected kind,
    /// with no diagnostics emitted.
    fn single_clean(src: &str) -> TokenKind {
        let (toks, diags) = lex_with_diags(src);
        assert!(
            diags.is_empty(),
            "unexpected diagnostics for `{src}`: {diags:?}"
        );
        assert_eq!(
            toks.len(),
            1,
            "expected one token for `{src}`, got {toks:?}"
        );
        toks[0].kind.clone()
    }

    /// Extract a [`CharLiteral`] value + prefix, failing loudly if the token
    /// is not a character literal.
    fn as_char(k: &TokenKind) -> (u32, CharPrefix) {
        match k {
            TokenKind::CharLiteral { value, prefix } => (*value, *prefix),
            other => panic!("expected CharLiteral, got {other:?}"),
        }
    }

    /// Extract a [`StringLiteral`] value + prefix.
    fn as_string(k: &TokenKind) -> (String, StringPrefix) {
        match k {
            TokenKind::StringLiteral { value, prefix } => (value.clone(), *prefix),
            other => panic!("expected StringLiteral, got {other:?}"),
        }
    }

    // =====================================================================
    // Character literals
    // =====================================================================

    // ---------- Basic shapes and prefixes ----------

    #[test]
    fn simple_char_literal() {
        let (v, p) = as_char(&single_clean("'A'"));
        assert_eq!(v, 0x41);
        assert_eq!(p, CharPrefix::None);
    }

    #[test]
    fn wide_l_char_literal() {
        let (v, p) = as_char(&single_clean("L'A'"));
        assert_eq!(v, 0x41);
        assert_eq!(p, CharPrefix::L);
    }

    #[test]
    fn utf16_lowercase_u_char_literal() {
        let (v, p) = as_char(&single_clean("u'A'"));
        assert_eq!(v, 0x41);
        assert_eq!(p, CharPrefix::U16);
    }

    #[test]
    fn utf32_uppercase_u_char_literal() {
        let (v, p) = as_char(&single_clean("U'A'"));
        assert_eq!(v, 0x41);
        assert_eq!(p, CharPrefix::U32);
    }

    #[test]
    fn char_literal_span_includes_prefix_and_quotes() {
        let (toks, diags) = lex_with_diags("L'A'");
        assert!(diags.is_empty());
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].span, Span::new(0, 4));
    }

    // ---------- Simple escape sequences (every one) ----------

    #[test]
    fn all_simple_escape_sequences() {
        let cases: &[(&str, u32)] = &[
            (r"'\a'", 0x07),
            (r"'\b'", 0x08),
            (r"'\f'", 0x0C),
            (r"'\n'", 0x0A),
            (r"'\r'", 0x0D),
            (r"'\t'", 0x09),
            (r"'\v'", 0x0B),
            (r"'\\'", 0x5C),
            (r"'\''", 0x27),
            (r#"'\"'"#, 0x22),
            (r"'\?'", 0x3F),
        ];
        for (src, expected) in cases {
            let (v, p) = as_char(&single_clean(src));
            assert_eq!(v, *expected, "mis-lexed escape `{src}`");
            assert_eq!(p, CharPrefix::None);
        }
    }

    // ---------- Octal escapes ----------

    #[test]
    fn octal_null_escape() {
        let (v, _) = as_char(&single_clean(r"'\0'"));
        assert_eq!(v, 0);
    }

    #[test]
    fn octal_escape_one_digit() {
        let (v, _) = as_char(&single_clean(r"'\7'"));
        assert_eq!(v, 0o7);
    }

    #[test]
    fn octal_escape_two_digits() {
        let (v, _) = as_char(&single_clean(r"'\12'"));
        assert_eq!(v, 0o12);
    }

    #[test]
    fn octal_escape_three_digits() {
        let (v, _) = as_char(&single_clean(r"'\377'"));
        assert_eq!(v, 0o377);
    }

    #[test]
    fn octal_escape_stops_at_three_digits() {
        // `\1234` is `\123` (value 0o123 = 83) followed by `'4'` (0x34 = 52).
        // So `'\1234'` is a multi-character narrow constant: (0o123 << 8) | '4'.
        let (v, _) = as_char(&single_clean(r"'\1234'"));
        assert_eq!(v, (0o123u32 << 8) | b'4' as u32);
    }

    #[test]
    fn octal_escape_stops_at_non_octal_digit() {
        // `\19` is octal `\1` (value 1) then literal `9`.
        let (v, _) = as_char(&single_clean(r"'\19'"));
        assert_eq!(v, (1u32 << 8) | b'9' as u32);
    }

    #[test]
    fn octal_escape_boundary_zero_and_max() {
        let (v0, _) = as_char(&single_clean(r"'\0'"));
        assert_eq!(v0, 0);
        let (vmax, _) = as_char(&single_clean(r"'\377'"));
        assert_eq!(vmax, 255);
    }

    // ---------- Hex escapes ----------

    #[test]
    fn hex_escape_ascii_a() {
        let (v, _) = as_char(&single_clean(r"'\x41'"));
        assert_eq!(v, 0x41);
    }

    #[test]
    fn hex_escape_lowercase_and_uppercase_mixed() {
        let (v, _) = as_char(&single_clean(r"'\xFf'"));
        assert_eq!(v, 0xFF);
    }

    #[test]
    fn hex_escape_one_digit() {
        let (v, _) = as_char(&single_clean(r"'\x1'"));
        assert_eq!(v, 0x1);
    }

    #[test]
    fn hex_escape_many_digits_in_wide_literal() {
        // Wide-literal prefix lets the value exceed 8 bits cleanly.
        let (v, p) = as_char(&single_clean(r"U'\x12345678'"));
        assert_eq!(v, 0x1234_5678);
        assert_eq!(p, CharPrefix::U32);
    }

    #[test]
    fn hex_escape_stops_at_non_hex() {
        // `"\x41g"` is byte 0x41 ('A') then literal 'g'.
        let (s, _) = as_string(&single_clean(r#""\x41g""#));
        assert_eq!(s, "Ag");
    }

    #[test]
    fn hex_escape_without_digits_is_error() {
        let (toks, diags) = lex_with_diags(r#""\x""#);
        assert_eq!(toks.len(), 1);
        assert_eq!(as_string(&toks[0].kind).0, "");
        assert!(diags.iter().any(|d| d.message.contains("\\x")));
    }

    #[test]
    fn hex_escape_overflow_emits_warning() {
        // 9 hex digits: 0x123456789 overflows 32 bits.
        let (_, diags) = lex_with_diags(r"U'\x123456789'");
        assert!(
            diags.iter().any(|d| d.message.contains("overflow")),
            "expected overflow warning for 9-digit hex escape, got {diags:?}"
        );
    }

    // ---------- Universal character names ----------

    #[test]
    fn ucn_u_exact_four_digits() {
        let (v, _) = as_char(&single_clean(r"U'\u4e2d'"));
        assert_eq!(v, 0x4E2D);
    }

    #[test]
    fn ucn_big_u_exact_eight_digits() {
        let (v, _) = as_char(&single_clean(r"U'\U0001F600'"));
        assert_eq!(v, 0x1_F600);
    }

    #[test]
    fn ucn_u_with_too_few_digits_is_error() {
        let (_, diags) = lex_with_diags(r"'\u12'");
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("universal character name")),
            "got diagnostics: {diags:?}"
        );
    }

    #[test]
    fn ucn_big_u_truncated_by_quote_is_error() {
        // `\U000041'` has only 6 hex digits before the closing quote.
        let (_, diags) = lex_with_diags(r"'\U000041'");
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("universal character name")),
            "got diagnostics: {diags:?}"
        );
    }

    #[test]
    fn ucn_with_non_hex_digit_is_error() {
        let (_, diags) = lex_with_diags(r"'\u00Gz'");
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("universal character name")),
            "got diagnostics: {diags:?}"
        );
    }

    #[test]
    fn ucn_encoded_into_utf8_string() {
        let (s, _) = as_string(&single_clean(r#""\u4e2d""#));
        // 0x4E2D is the Chinese character 中 — three bytes in UTF-8.
        assert_eq!(s, "\u{4e2d}");
        assert_eq!(s.len(), 3);
    }

    // ---------- Empty / multi-character / boundary cases ----------

    #[test]
    fn empty_char_literal_is_error() {
        let (toks, diags) = lex_with_diags("''");
        assert_eq!(toks.len(), 1);
        let (v, p) = as_char(&toks[0].kind);
        assert_eq!(v, 0, "empty literal still produces a token, value = 0");
        assert_eq!(p, CharPrefix::None);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("empty character constant")),
            "got diagnostics: {diags:?}"
        );
    }

    #[test]
    fn multi_character_narrow_constant_packs_bytes() {
        // 'ab' is implementation-defined but valid — GCC packs 'a' into
        // the high byte.  value = ('a' << 8) | 'b' = 0x6162 = 24930.
        let (v, p) = as_char(&single_clean("'ab'"));
        assert_eq!(v, 0x6162);
        assert_eq!(p, CharPrefix::None);
    }

    #[test]
    fn multi_character_with_escapes() {
        // '\n\t' packs \n (0x0A) into the high byte and \t (0x09) in low.
        let (v, _) = as_char(&single_clean(r"'\n\t'"));
        assert_eq!(v, (0x0A_u32 << 8) | 0x09);
    }

    #[test]
    fn four_byte_multichar_constant() {
        let (v, _) = as_char(&single_clean("'abcd'"));
        assert_eq!(
            v,
            (b'a' as u32) << 24 | (b'b' as u32) << 16 | (b'c' as u32) << 8 | b'd' as u32
        );
    }

    #[test]
    fn multi_character_wide_constant_warns_and_returns_first() {
        let (toks, diags) = lex_with_diags("L'ab'");
        assert_eq!(toks.len(), 1);
        let (v, p) = as_char(&toks[0].kind);
        assert_eq!(v, b'a' as u32);
        assert_eq!(p, CharPrefix::L);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("multi-character wide")),
            "got diagnostics: {diags:?}"
        );
    }

    // ---------- Error recovery ----------

    #[test]
    fn unterminated_char_literal_at_eof() {
        let (toks, diags) = lex_with_diags("'abc");
        assert_eq!(toks.len(), 1);
        let (v, _) = as_char(&toks[0].kind);
        // Three narrow chars packed: 'a','b','c'.
        assert_eq!(v, (b'a' as u32) << 16 | (b'b' as u32) << 8 | b'c' as u32);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("unterminated character constant")),
            "got diagnostics: {diags:?}"
        );
    }

    #[test]
    fn unterminated_char_literal_stops_at_newline() {
        // Scanner must not gobble the newline: subsequent tokens must
        // appear on the next line with `at_start_of_line` set.
        let (toks, diags) = lex_with_diags("'abc\nx");
        assert_eq!(toks.len(), 2);
        assert!(matches!(toks[0].kind, TokenKind::CharLiteral { .. }));
        assert_eq!(toks[1].kind, TokenKind::Identifier("x".to_string()));
        assert!(toks[1].at_start_of_line);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("unterminated character constant")),
            "got diagnostics: {diags:?}"
        );
    }

    #[test]
    fn invalid_escape_emits_diagnostic_but_literal_survives() {
        let (toks, diags) = lex_with_diags(r"'\q'");
        assert_eq!(toks.len(), 1);
        let (v, _) = as_char(&toks[0].kind);
        assert_eq!(
            v, b'q' as u32,
            "invalid escape falls back to literal char value"
        );
        assert!(
            diags.iter().any(|d| d.message.contains("unknown escape")),
            "got diagnostics: {diags:?}"
        );
    }

    #[test]
    fn backslash_newline_in_char_literal_is_accepted_as_continuation() {
        // '\<newline>a' = line continuation + 'a' = value 'a'.
        let (toks, diags) = lex_with_diags("'\\\na'");
        assert_eq!(toks.len(), 1);
        let (v, _) = as_char(&toks[0].kind);
        assert_eq!(v, b'a' as u32);
        assert!(diags.is_empty(), "no diagnostics expected, got {diags:?}");
    }

    #[test]
    fn lone_backslash_before_eof_in_char_literal() {
        let (toks, diags) = lex_with_diags(r"'\");
        assert_eq!(toks.len(), 1);
        assert!(
            diags.iter().any(|d| d.message.contains("incomplete escape")
                || d.message.contains("unterminated character constant")),
            "got diagnostics: {diags:?}"
        );
    }

    // =====================================================================
    // String literals
    // =====================================================================

    // ---------- Basic shapes and prefixes ----------

    #[test]
    fn simple_string_literal() {
        let (s, p) = as_string(&single_clean(r#""hello""#));
        assert_eq!(s, "hello");
        assert_eq!(p, StringPrefix::None);
    }

    #[test]
    fn empty_string_is_legal() {
        let (s, p) = as_string(&single_clean(r#""""#));
        assert_eq!(s, "");
        assert_eq!(p, StringPrefix::None);
    }

    #[test]
    fn wide_l_string_literal() {
        let (s, p) = as_string(&single_clean(r#"L"hello""#));
        assert_eq!(s, "hello");
        assert_eq!(p, StringPrefix::L);
    }

    #[test]
    fn utf8_string_literal() {
        let (s, p) = as_string(&single_clean(r#"u8"hello""#));
        assert_eq!(s, "hello");
        assert_eq!(p, StringPrefix::Utf8);
    }

    #[test]
    fn utf16_string_literal() {
        let (s, p) = as_string(&single_clean(r#"u"hello""#));
        assert_eq!(s, "hello");
        assert_eq!(p, StringPrefix::U16);
    }

    #[test]
    fn utf32_string_literal() {
        let (s, p) = as_string(&single_clean(r#"U"hello""#));
        assert_eq!(s, "hello");
        assert_eq!(p, StringPrefix::U32);
    }

    #[test]
    fn string_literal_span_includes_prefix_and_quotes() {
        let (toks, _) = lex_with_diags(r#"u8"hi""#);
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].span, Span::new(0, 6));
    }

    // ---------- Escape sequences in strings ----------

    #[test]
    fn all_simple_escapes_in_string() {
        let (s, _) = as_string(&single_clean(r#""\a\b\f\n\r\t\v\\\'\"\?""#));
        assert_eq!(s, "\u{07}\u{08}\u{0C}\n\r\t\u{0B}\\\'\"?");
    }

    #[test]
    fn octal_and_hex_escapes_in_string() {
        let (s, _) = as_string(&single_clean(r#""\101\x42""#));
        assert_eq!(s, "AB");
    }

    #[test]
    fn universal_character_names_in_string() {
        let (s, _) = as_string(&single_clean(r#""\u00e9\U0001F600""#));
        assert_eq!(s, "\u{e9}\u{1F600}");
    }

    // ---------- Line continuation ----------

    #[test]
    fn line_continuation_joins_parts_of_string() {
        // "hello\<NL>world" — the backslash + newline should vanish.
        let (s, _) = as_string(&single_clean("\"hello\\\nworld\""));
        assert_eq!(s, "helloworld");
    }

    #[test]
    fn line_continuation_with_crlf() {
        let (s, _) = as_string(&single_clean("\"ab\\\r\ncd\""));
        assert_eq!(s, "abcd");
    }

    // ---------- Error recovery ----------

    #[test]
    fn unterminated_string_at_eof() {
        let (toks, diags) = lex_with_diags(r#""hello"#);
        assert_eq!(toks.len(), 1);
        let (s, _) = as_string(&toks[0].kind);
        assert_eq!(s, "hello");
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("unterminated string literal")),
            "got diagnostics: {diags:?}"
        );
    }

    #[test]
    fn unterminated_string_stops_at_newline() {
        let (toks, diags) = lex_with_diags("\"hello\nworld\"");
        // First token: unterminated StringLiteral with body "hello".
        // Then `world`, then another unterminated string.
        assert!(!toks.is_empty());
        let (s0, _) = as_string(&toks[0].kind);
        assert_eq!(s0, "hello");
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("unterminated string literal")),
            "got diagnostics: {diags:?}"
        );
        // The identifier `world` must appear on the next line.
        let world = toks
            .iter()
            .find(|t| matches!(&t.kind, TokenKind::Identifier(s) if s == "world"));
        assert!(
            world.is_some(),
            "expected `world` identifier after unterminated string, got {toks:?}"
        );
        assert!(world.unwrap().at_start_of_line);
    }

    #[test]
    fn invalid_escape_in_string_emits_warning_but_keeps_literal() {
        let (toks, diags) = lex_with_diags(r#""\q""#);
        assert_eq!(toks.len(), 1);
        let (s, _) = as_string(&toks[0].kind);
        assert_eq!(s, "q");
        assert!(
            diags.iter().any(|d| d.message.contains("unknown escape")),
            "got diagnostics: {diags:?}"
        );
    }

    #[test]
    fn string_with_leading_newline_produces_empty_string_and_diagnostic() {
        // `"` alone on a line, then newline — unterminated.
        let (toks, diags) = lex_with_diags("\"\n");
        assert!(!toks.is_empty());
        let (s, _) = as_string(&toks[0].kind);
        assert!(s.is_empty());
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("unterminated string literal")),
            "got diagnostics: {diags:?}"
        );
    }

    // ---------- Cross-cutting: prefixes must not be mis-lexed as identifiers ----------

    #[test]
    fn l_alone_is_still_an_identifier() {
        // `L` without a following quote is just the identifier `L`.
        let (toks, diags) = lex_with_diags("L");
        assert!(diags.is_empty());
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TokenKind::Identifier("L".to_string()));
    }

    #[test]
    fn u_followed_by_ident_is_identifier() {
        // `u8abc` is a single identifier, not `u8` + identifier.
        let (toks, diags) = lex_with_diags("u8abc");
        assert!(diags.is_empty());
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TokenKind::Identifier("u8abc".to_string()));
    }

    #[test]
    fn u8_with_apostrophe_is_identifier_then_char_literal() {
        // u8 is NOT a valid char-literal prefix — only a string prefix.
        // So `u8'x'` must lex as identifier `u8` then char literal `'x'`.
        let (toks, diags) = lex_with_diags("u8'x'");
        assert!(diags.is_empty(), "unexpected diags: {diags:?}");
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].kind, TokenKind::Identifier("u8".to_string()));
        let (v, p) = as_char(&toks[1].kind);
        assert_eq!(v, b'x' as u32);
        assert_eq!(p, CharPrefix::None);
    }

    #[test]
    fn adjacent_string_literals_produce_two_tokens() {
        // The lexer does not concatenate adjacent literals; that is a
        // translation-phase-6 job handled by the preprocessor/parser.
        let (toks, diags) = lex_with_diags(r#""foo" "bar""#);
        assert!(diags.is_empty());
        assert_eq!(toks.len(), 2);
        assert_eq!(as_string(&toks[0].kind).0, "foo");
        assert_eq!(as_string(&toks[1].kind).0, "bar");
    }

    // ---------- Boundary: all prefixes × a representative escape ----------

    #[test]
    fn every_char_prefix_accepts_hex_escape() {
        for src in &[r"'\x41'", r"L'\x41'", r"u'\x41'", r"U'\x41'"] {
            let (v, _) = as_char(&single_clean(src));
            assert_eq!(v, 0x41, "mis-lexed `{src}`");
        }
    }

    #[test]
    fn every_string_prefix_accepts_all_escape_families() {
        for src in &[
            r#""\n\x41\101\u00e9""#,
            r#"L"\n\x41\101\u00e9""#,
            r#"u8"\n\x41\101\u00e9""#,
            r#"u"\n\x41\101\u00e9""#,
            r#"U"\n\x41\101\u00e9""#,
        ] {
            let (s, _) = as_string(&single_clean(src));
            assert_eq!(s, "\nAA\u{e9}", "mis-lexed `{src}`");
        }
    }
}
