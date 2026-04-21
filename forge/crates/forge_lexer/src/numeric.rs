//! Numeric literal sub-lexer.
//!
//! The single public entry point is [`Lexer::lex_numeric_literal`].  It
//! is dispatched to from [`Lexer::lex_kind`](crate::lexer::Lexer) when
//! the current byte is a digit, or when a `.` is immediately followed
//! by a digit (fractional-only decimal floats such as `.5`).
//!
//! # Grammar covered
//!
//! ```text
//! integer-literal := decimal-integer | octal-integer | hex-integer
//!
//! decimal-integer := nonzero-digit digit* int-suffix?
//!                  | "0" int-suffix?
//! octal-integer   := "0" octal-digit+ int-suffix?
//! hex-integer     := "0" ("x" | "X") hex-digit+ int-suffix?
//!
//! float-literal   := decimal-float | hex-float
//! decimal-float   := digit+ "." digit* exponent? float-suffix?
//!                  | "." digit+ exponent? float-suffix?
//!                  | digit+ exponent float-suffix?
//! hex-float       := "0" ("x" | "X") hex-digit* ("." hex-digit*)? p-exp float-suffix?
//!
//! int-suffix   := one of u, U, l, L, ll, LL, plus optional u/U, in any order
//! float-suffix := one of f, F, l, L
//! exponent     := ("e" | "E") ("+" | "-")? digit+
//! p-exp        := ("p" | "P") ("+" | "-")? digit+
//! ```
//!
//! # Error recovery
//!
//! Malformed numbers always produce a token (with the best value
//! recoverable) and emit a [`Diagnostic`] on the lexer.  The concrete
//! recoveries are:
//!
//! * **Integer overflow** (value > `u64::MAX`) — warning; the value is
//!   the low 64 bits.
//! * **Invalid octal digit** (`08`, `09`) — error; the digits are still
//!   consumed and interpreted as if they were octal, but the overall
//!   value is unreliable.
//! * **Empty hex integer** (`0x` alone) — error; value `0` is returned.
//! * **Hex float without binary exponent** (`0x1.5`) — error; the
//!   exponent is treated as `0`.
//! * **Exponent without digits** (`1e`, `0x1p`) — error; the exponent
//!   is treated as `0`.

use forge_diagnostics::{Diagnostic, Span};

use crate::lexer::Lexer;
use crate::token::{FloatSuffix, IntSuffix, TokenKind};

impl Lexer<'_> {
    // ---------------------------------------------------------------------
    // Public entry point.
    // ---------------------------------------------------------------------

    /// Lex a numeric literal at the current cursor.
    ///
    /// Precondition: `self.peek()` is `Some(d)` where either `d` is a
    /// digit, or `d == b'.'` and `peek_at(1)` is a digit (fractional-only
    /// float).  Returns an [`IntegerLiteral`](TokenKind::IntegerLiteral)
    /// or a [`FloatLiteral`](TokenKind::FloatLiteral) depending on what
    /// it finds.
    pub(crate) fn lex_numeric_literal(&mut self) -> TokenKind {
        let start = self.pos;

        // `.digit` — decimal float with no integer part.
        if self.peek() == Some(b'.') {
            return self.lex_float_starting_with_dot(start);
        }

        // `0x` or `0X` — hex integer or hex float.
        if self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'x' | b'X')) {
            return self.lex_hex_number(start);
        }

        // Decimal integer, octal integer, or decimal float.
        self.lex_decimal_number(start)
    }

    // ---------------------------------------------------------------------
    // Decimal integers and decimal floats.
    // ---------------------------------------------------------------------

    /// Handle a decimal digit run — which may turn out to be a decimal
    /// integer, an octal integer, or a decimal float.
    fn lex_decimal_number(&mut self, start: usize) -> TokenKind {
        let digits_start = self.pos;
        let is_leading_zero = self.peek() == Some(b'0');
        self.consume_decimal_digits();
        let int_digits_end = self.pos;

        // Decide whether the integer run is actually the leading part of
        // a decimal float.
        if self.next_is_decimal_float_tail() {
            return self.lex_decimal_float_tail(start, digits_start);
        }

        // Integer path.
        let text = &self.source[digits_start..int_digits_end];
        let value = if is_leading_zero && text.len() > 1 {
            self.parse_octal_integer(text, start)
        } else {
            self.parse_decimal_integer(text, start)
        };

        let suffix = self.parse_int_suffix();
        TokenKind::IntegerLiteral { value, suffix }
    }

    /// Decide whether the current cursor (positioned immediately after
    /// the integer-part digits of a decimal number) starts the tail of a
    /// decimal float, implementing the `1.5` vs `1.method` rule.
    fn next_is_decimal_float_tail(&self) -> bool {
        match self.peek() {
            Some(b'.') => match self.peek_at(1) {
                // `1.5` — digit after dot: definitely a float.
                // `1.e5` — exponent follows dot: float with empty
                // fractional part.
                Some(b'0'..=b'9' | b'e' | b'E') => true,
                // `1.method` or `1._x` — ident-start letter means the
                // dot belongs to member access, not the number.
                Some(b'_' | b'a'..=b'z' | b'A'..=b'Z') => false,
                // `1.` at EOF, `1.;`, `1.+`, `1..`, etc. — no ident-like
                // continuation, so treat as a trailing-dot float.
                _ => true,
            },
            // `1e5` — decimal float without a dot.
            Some(b'e' | b'E') => true,
            _ => false,
        }
    }

    /// After the integer digits have been consumed, finish the tail of a
    /// decimal float (`.fractional?` then optional exponent then optional
    /// suffix) and return the resulting token.
    fn lex_decimal_float_tail(&mut self, start: usize, body_start: usize) -> TokenKind {
        if self.peek() == Some(b'.') {
            self.pos += 1;
            self.consume_decimal_digits();
        }
        self.maybe_consume_decimal_exponent(start);

        let body_end = self.pos;
        let suffix = self.parse_float_suffix();
        let value = self.parse_f64_body(start, body_start, body_end);
        TokenKind::FloatLiteral { value, suffix }
    }

    /// Dispatched from `lex_numeric_literal` when the number begins with
    /// a bare `.` (e.g., `.5`).
    fn lex_float_starting_with_dot(&mut self, start: usize) -> TokenKind {
        debug_assert_eq!(self.peek(), Some(b'.'));
        self.pos += 1; // consume `.`
                       // Caller guaranteed a digit follows; consume the full run.
        self.consume_decimal_digits();
        self.maybe_consume_decimal_exponent(start);

        let body_end = self.pos;
        let suffix = self.parse_float_suffix();
        let value = self.parse_f64_body(start, start, body_end);
        TokenKind::FloatLiteral { value, suffix }
    }

    /// Consume an optional decimal exponent (`e` / `E`, optional sign,
    /// then required digits).  Emits a diagnostic if the exponent letter
    /// appears but no digits follow; the cursor is still advanced past
    /// the `e`/sign so the overall literal span is accurate.
    fn maybe_consume_decimal_exponent(&mut self, lit_start: usize) {
        if !matches!(self.peek(), Some(b'e' | b'E')) {
            return;
        }
        self.pos += 1; // consume e / E
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        let digits_start = self.pos;
        self.consume_decimal_digits();
        if self.pos == digits_start {
            self.emit_diagnostic(
                Diagnostic::error("exponent has no digits")
                    .span(Span::new(self.file_id, lit_start as u32, self.pos as u32))
                    .label("a decimal exponent must have at least one digit after `e`/`E`"),
            );
        }
    }

    // ---------------------------------------------------------------------
    // Hex integers and hex floats.
    // ---------------------------------------------------------------------

    /// Handle a `0x` / `0X` prefix — either a hex integer or a hex float.
    fn lex_hex_number(&mut self, start: usize) -> TokenKind {
        self.pos += 2; // consume `0x` or `0X`
        let int_start = self.pos;
        self.consume_hex_digits();
        let int_end = self.pos;
        let had_int_digits = int_end > int_start;

        // `.` or `p`/`P` after the hex digit run marks a hex float.  Note
        // that for hex we *always* treat `.` as float-entry (unlike the
        // decimal side, which has to disambiguate against member access)
        // because member access after a hex integer is vanishingly rare
        // and the ambiguity would mask malformed hex-float literals.
        let is_float = matches!(self.peek(), Some(b'.' | b'p' | b'P'));
        if is_float {
            return self.finish_hex_float(start, int_start, int_end, had_int_digits);
        }

        if !had_int_digits {
            self.emit_diagnostic(
                Diagnostic::error("hex integer literal has no digits")
                    .span(Span::new(self.file_id, start as u32, self.pos as u32))
                    .label("expected one or more hex digits after `0x`"),
            );
            let suffix = self.parse_int_suffix();
            return TokenKind::IntegerLiteral { value: 0, suffix };
        }

        let text = &self.source[int_start..int_end];
        let value = self.parse_hex_integer(text, start);
        let suffix = self.parse_int_suffix();
        TokenKind::IntegerLiteral { value, suffix }
    }

    /// Finish a hex float after the integer-part hex digits have been
    /// consumed.  The cursor is at `.` or `p`/`P`.
    fn finish_hex_float(
        &mut self,
        start: usize,
        int_start: usize,
        int_end: usize,
        had_int_digits: bool,
    ) -> TokenKind {
        let int_part = &self.source[int_start..int_end];
        let mut frac_part: &str = "";

        if self.peek() == Some(b'.') {
            self.pos += 1;
            let frac_start = self.pos;
            self.consume_hex_digits();
            frac_part = &self.source[frac_start..self.pos];
        }

        if !had_int_digits && frac_part.is_empty() {
            self.emit_diagnostic(
                Diagnostic::error("hex float literal has no hex digits")
                    .span(Span::new(self.file_id, start as u32, self.pos as u32))
                    .label("a hex float must have digits before or after the `.`"),
            );
        }

        let exponent = self.consume_hex_binary_exponent(start);
        let suffix = self.parse_float_suffix();
        let value = compute_hex_float(int_part, frac_part, exponent);
        TokenKind::FloatLiteral { value, suffix }
    }

    /// Consume the mandatory `p`/`P` binary exponent of a hex float.  On
    /// a missing or digit-less exponent a diagnostic is emitted and the
    /// returned exponent is `0` — the value still round-trips as a float,
    /// just with the wrong magnitude (the error message is the real
    /// signal to the user).
    fn consume_hex_binary_exponent(&mut self, lit_start: usize) -> i32 {
        let Some(c) = self.peek() else {
            self.emit_diagnostic(
                Diagnostic::error("hex float missing binary exponent")
                    .span(Span::new(self.file_id, lit_start as u32, self.pos as u32))
                    .label("hex floating-point literals require a `p` binary exponent"),
            );
            return 0;
        };
        if c != b'p' && c != b'P' {
            self.emit_diagnostic(
                Diagnostic::error("hex float missing binary exponent")
                    .span(Span::new(self.file_id, lit_start as u32, self.pos as u32))
                    .label("hex floating-point literals require a `p` binary exponent"),
            );
            return 0;
        }
        self.pos += 1; // consume p / P

        let mut negate = false;
        match self.peek() {
            Some(b'+') => self.pos += 1,
            Some(b'-') => {
                negate = true;
                self.pos += 1;
            }
            _ => {}
        }

        let digits_start = self.pos;
        self.consume_decimal_digits();
        if self.pos == digits_start {
            self.emit_diagnostic(
                Diagnostic::error("hex float exponent has no digits")
                    .span(Span::new(self.file_id, lit_start as u32, self.pos as u32))
                    .label("the binary exponent (`p...`) must have at least one digit"),
            );
            return 0;
        }

        let digits = &self.source[digits_start..self.pos];
        // Saturate into `i32`: the practical useful range is small and
        // 2^(±32_768) already over/underflows f64.  Anything that doesn't
        // fit in i32 is clamped — not ideal, but the literal is unusable
        // at that magnitude anyway.
        let magnitude: i32 = digits.parse().unwrap_or(i32::MAX);
        if negate {
            magnitude.saturating_neg()
        } else {
            magnitude
        }
    }

    // ---------------------------------------------------------------------
    // Digit-run helpers.
    // ---------------------------------------------------------------------

    fn consume_decimal_digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
    }

    fn consume_hex_digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F')) {
            self.pos += 1;
        }
    }

    // ---------------------------------------------------------------------
    // Integer value parsing (with overflow + invalid-digit diagnostics).
    // ---------------------------------------------------------------------

    fn parse_decimal_integer(&mut self, text: &str, span_start: usize) -> u64 {
        let mut value: u64 = 0;
        let mut overflowed = false;
        for c in text.bytes() {
            let d = (c - b'0') as u64;
            let (v1, o1) = value.overflowing_mul(10);
            let (v2, o2) = v1.overflowing_add(d);
            overflowed |= o1 | o2;
            value = v2;
        }
        if overflowed {
            self.emit_integer_overflow(span_start);
        }
        value
    }

    fn parse_octal_integer(&mut self, text: &str, span_start: usize) -> u64 {
        let mut value: u64 = 0;
        let mut overflowed = false;
        let mut has_invalid_digit = false;
        for c in text.bytes() {
            if !matches!(c, b'0'..=b'7') {
                has_invalid_digit = true;
            }
            let d = (c - b'0') as u64;
            let (v1, o1) = value.overflowing_mul(8);
            let (v2, o2) = v1.overflowing_add(d);
            overflowed |= o1 | o2;
            value = v2;
        }
        if has_invalid_digit {
            self.emit_diagnostic(
                Diagnostic::error("invalid digit in octal literal")
                    .span(Span::new(self.file_id, span_start as u32, self.pos as u32))
                    .label("octal literals may only contain digits 0–7")
                    .note("a literal starting with `0` is octal; use `0x...` for hex"),
            );
        }
        if overflowed {
            self.emit_integer_overflow(span_start);
        }
        value
    }

    fn parse_hex_integer(&mut self, text: &str, span_start: usize) -> u64 {
        let mut value: u64 = 0;
        let mut overflowed = false;
        for c in text.bytes() {
            let d = hex_digit_value(c) as u64;
            let (v1, o1) = value.overflowing_mul(16);
            let (v2, o2) = v1.overflowing_add(d);
            overflowed |= o1 | o2;
            value = v2;
        }
        if overflowed {
            self.emit_integer_overflow(span_start);
        }
        value
    }

    fn emit_integer_overflow(&mut self, span_start: usize) {
        self.emit_diagnostic(
            Diagnostic::warning("integer literal is too large to fit in 64 bits")
                .span(Span::new(self.file_id, span_start as u32, self.pos as u32))
                .label("value has been truncated to its low 64 bits"),
        );
    }

    // ---------------------------------------------------------------------
    // Float value parsing.
    // ---------------------------------------------------------------------

    /// Parse the substring `source[body_start..body_end]` as an `f64` using
    /// Rust's standard parser, emitting a diagnostic if the parse fails.
    ///
    /// `body_start` points at the first character that belongs to the
    /// floating-point body (digits or leading `.`); `body_end` is just
    /// past the last such character — in particular, before any `f`/`l`
    /// suffix.  `lit_start` is used only for the diagnostic span.
    fn parse_f64_body(&mut self, lit_start: usize, body_start: usize, body_end: usize) -> f64 {
        let body = &self.source[body_start..body_end];
        if let Ok(v) = body.parse::<f64>() {
            v
        } else {
            self.emit_diagnostic(
                Diagnostic::error(format!("invalid floating-point literal `{body}`"))
                    .span(Span::new(self.file_id, lit_start as u32, self.pos as u32))
                    .label("could not parse as a floating-point value"),
            );
            0.0
        }
    }

    // ---------------------------------------------------------------------
    // Suffix parsing.
    // ---------------------------------------------------------------------

    /// Parse a trailing integer suffix.
    ///
    /// Accepts every C17 valid combination:
    ///
    /// * `u` / `U` alone  → [`IntSuffix::U`]
    /// * `l` / `L` alone  → [`IntSuffix::L`]
    /// * `ll` / `LL`      → [`IntSuffix::LL`] (case-matching pairs only)
    /// * `u` with `l`/`L` in either order → [`IntSuffix::UL`]
    /// * `u` with `ll`/`LL` in either order → [`IntSuffix::ULL`]
    ///
    /// The matcher is longest-match: it prefers `ull` over `ul`, and
    /// `ul` over `u`, so ambiguous inputs like `1ull` are parsed as a
    /// single suffix rather than `1u` + identifier `ll`.
    fn parse_int_suffix(&mut self) -> IntSuffix {
        // Longest-match table — 3-byte suffixes are tried first so that
        // e.g. `ull` is not decomposed as `u` + `ll`.
        const LONG_SUFFIXES: &[(&[u8], IntSuffix)] = &[
            (b"ull", IntSuffix::ULL),
            (b"uLL", IntSuffix::ULL),
            (b"Ull", IntSuffix::ULL),
            (b"ULL", IntSuffix::ULL),
            (b"llu", IntSuffix::ULL),
            (b"llU", IntSuffix::ULL),
            (b"LLu", IntSuffix::ULL),
            (b"LLU", IntSuffix::ULL),
        ];
        const MED_SUFFIXES: &[(&[u8], IntSuffix)] = &[
            (b"ul", IntSuffix::UL),
            (b"uL", IntSuffix::UL),
            (b"Ul", IntSuffix::UL),
            (b"UL", IntSuffix::UL),
            (b"lu", IntSuffix::UL),
            (b"lU", IntSuffix::UL),
            (b"Lu", IntSuffix::UL),
            (b"LU", IntSuffix::UL),
            (b"ll", IntSuffix::LL),
            (b"LL", IntSuffix::LL),
        ];
        const SHORT_SUFFIXES: &[(&[u8], IntSuffix)] = &[
            (b"u", IntSuffix::U),
            (b"U", IntSuffix::U),
            (b"l", IntSuffix::L),
            (b"L", IntSuffix::L),
        ];

        let remaining = &self.bytes[self.pos..];
        for table in [LONG_SUFFIXES, MED_SUFFIXES, SHORT_SUFFIXES] {
            for (pat, suffix) in table {
                if remaining.starts_with(pat) {
                    self.pos += pat.len();
                    return *suffix;
                }
            }
        }
        IntSuffix::None
    }

    /// Parse a trailing floating-point suffix.
    ///
    /// Only a single suffix letter is consumed: `f`/`F` → `F`,
    /// `l`/`L` → `L`.  Anything else leaves the cursor untouched.
    fn parse_float_suffix(&mut self) -> FloatSuffix {
        match self.peek() {
            Some(b'f' | b'F') => {
                self.pos += 1;
                FloatSuffix::F
            }
            Some(b'l' | b'L') => {
                self.pos += 1;
                FloatSuffix::L
            }
            _ => FloatSuffix::None,
        }
    }
}

// -------------------------------------------------------------------------
// Free helpers.
// -------------------------------------------------------------------------

/// Map a hex digit byte to its numeric value `0..=15`.  Panics on a
/// non-hex byte, which is unreachable because every caller only invokes
/// this after [`Lexer::consume_hex_digits`] has validated the character
/// class.
fn hex_digit_value(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => unreachable!("non-hex digit reached hex_digit_value"),
    }
}

/// Compute the `f64` value of a hex float from its integer-part digits,
/// its fractional-part digits, and its binary exponent.
///
/// The computation is `(int_part . frac_part)_16 × 2^exponent`,
/// evaluated in `f64` arithmetic.  Accuracy is bounded by `f64`'s
/// 53-bit mantissa; inputs that cannot be represented exactly round to
/// the nearest representable value.
fn compute_hex_float(int_part: &str, frac_part: &str, exponent: i32) -> f64 {
    let mut mantissa: f64 = 0.0;
    for c in int_part.bytes() {
        mantissa = mantissa * 16.0 + hex_digit_value(c) as f64;
    }
    let mut scale = 1.0_f64 / 16.0;
    for c in frac_part.bytes() {
        mantissa += hex_digit_value(c) as f64 * scale;
        scale /= 16.0;
    }
    mantissa * 2_f64.powi(exponent)
}
