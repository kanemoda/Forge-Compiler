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

use forge_diagnostics::Diagnostic;

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
                    .span(lit_start..self.pos)
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
                    .span(start..self.pos)
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
                    .span(start..self.pos)
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
                    .span(lit_start..self.pos)
                    .label("hex floating-point literals require a `p` binary exponent"),
            );
            return 0;
        };
        if c != b'p' && c != b'P' {
            self.emit_diagnostic(
                Diagnostic::error("hex float missing binary exponent")
                    .span(lit_start..self.pos)
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
                    .span(lit_start..self.pos)
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
                    .span(span_start..self.pos)
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
                .span(span_start..self.pos)
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
                    .span(lit_start..self.pos)
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

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Lexer, Span, Token};

    // ---------- Helpers ----------

    fn lex_with_diags(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
        let mut lx = Lexer::new(src);
        let mut toks = lx.tokenize();
        let last = toks.pop().expect("tokenize always yields Eof");
        assert!(
            matches!(last.kind, TokenKind::Eof),
            "last token must be Eof"
        );
        let diags = lx.take_diagnostics();
        (toks, diags)
    }

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

    fn as_int(k: &TokenKind) -> (u64, IntSuffix) {
        match k {
            TokenKind::IntegerLiteral { value, suffix } => (*value, *suffix),
            other => panic!("expected IntegerLiteral, got {other:?}"),
        }
    }

    fn as_float(k: &TokenKind) -> (f64, FloatSuffix) {
        match k {
            TokenKind::FloatLiteral { value, suffix } => (*value, *suffix),
            other => panic!("expected FloatLiteral, got {other:?}"),
        }
    }

    // =====================================================================
    // Decimal integers
    // =====================================================================

    #[test]
    fn single_zero_is_decimal_zero() {
        // "0" is decimal, not octal — it only has one digit.
        let (v, s) = as_int(&single_clean("0"));
        assert_eq!(v, 0);
        assert_eq!(s, IntSuffix::None);
    }

    #[test]
    fn small_decimal_integer() {
        let (v, s) = as_int(&single_clean("42"));
        assert_eq!(v, 42);
        assert_eq!(s, IntSuffix::None);
    }

    #[test]
    fn large_decimal_integer() {
        let (v, _) = as_int(&single_clean("1234567890"));
        assert_eq!(v, 1_234_567_890);
    }

    #[test]
    fn u64_max_decimal() {
        let (v, _) = as_int(&single_clean("18446744073709551615"));
        assert_eq!(v, u64::MAX);
    }

    #[test]
    fn decimal_overflow_emits_warning() {
        let (_, diags) = lex_with_diags("18446744073709551616"); // u64::MAX + 1
        assert!(
            diags.iter().any(|d| d.message.contains("too large")),
            "expected overflow warning, got {diags:?}"
        );
    }

    #[test]
    fn very_long_decimal_overflow() {
        let (_, diags) = lex_with_diags("99999999999999999999999999999999");
        assert!(diags.iter().any(|d| d.message.contains("too large")));
    }

    // =====================================================================
    // Octal integers
    // =====================================================================

    #[test]
    fn two_digit_octal() {
        // `010` is octal 8.
        let (v, _) = as_int(&single_clean("010"));
        assert_eq!(v, 8);
    }

    #[test]
    fn three_digit_octal() {
        // `0777` is octal 511.
        let (v, _) = as_int(&single_clean("0777"));
        assert_eq!(v, 0o777);
    }

    #[test]
    fn octal_with_leading_zeros() {
        let (v, _) = as_int(&single_clean("0007"));
        assert_eq!(v, 7);
    }

    #[test]
    fn octal_invalid_digit_eight_emits_error() {
        let (_, diags) = lex_with_diags("08");
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("invalid digit in octal")),
            "expected invalid-octal error, got {diags:?}"
        );
    }

    #[test]
    fn octal_invalid_digit_nine_emits_error() {
        let (_, diags) = lex_with_diags("09");
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("invalid digit in octal")),
            "expected invalid-octal error, got {diags:?}"
        );
    }

    #[test]
    fn leading_zero_then_dot_is_float_not_octal() {
        // `08.5` is a decimal float 8.5 — no octal error.
        let (toks, diags) = lex_with_diags("08.5");
        assert!(diags.is_empty(), "unexpected diags: {diags:?}");
        assert_eq!(toks.len(), 1);
        let (v, _) = as_float(&toks[0].kind);
        assert!((v - 8.5).abs() < 1e-12);
    }

    // =====================================================================
    // Hex integers
    // =====================================================================

    #[test]
    fn hex_lowercase_x() {
        let (v, _) = as_int(&single_clean("0x1F"));
        assert_eq!(v, 31);
    }

    #[test]
    fn hex_uppercase_x() {
        let (v, _) = as_int(&single_clean("0Xdead"));
        assert_eq!(v, 0xDEAD);
    }

    #[test]
    fn hex_mixed_case_digits() {
        let (v, _) = as_int(&single_clean("0xCaFeBaBe"));
        assert_eq!(v, 0xCAFE_BABE);
    }

    #[test]
    fn hex_max_u64() {
        let (v, _) = as_int(&single_clean("0xFFFFFFFFFFFFFFFF"));
        assert_eq!(v, u64::MAX);
    }

    #[test]
    fn hex_overflow_emits_warning() {
        let (_, diags) = lex_with_diags("0x10000000000000000"); // 2^64
        assert!(
            diags.iter().any(|d| d.message.contains("too large")),
            "expected overflow warning, got {diags:?}"
        );
    }

    #[test]
    fn empty_hex_emits_error() {
        let (_, diags) = lex_with_diags("0x");
        assert!(
            diags.iter().any(|d| d.message.contains("no digits")),
            "expected no-digits error, got {diags:?}"
        );
    }

    // =====================================================================
    // Integer suffixes
    // =====================================================================

    #[test]
    fn suffix_u_both_cases() {
        assert_eq!(as_int(&single_clean("1u")).1, IntSuffix::U);
        assert_eq!(as_int(&single_clean("1U")).1, IntSuffix::U);
    }

    #[test]
    fn suffix_l_both_cases() {
        assert_eq!(as_int(&single_clean("1l")).1, IntSuffix::L);
        assert_eq!(as_int(&single_clean("1L")).1, IntSuffix::L);
    }

    #[test]
    fn suffix_ul_every_order_and_case() {
        for src in ["1ul", "1uL", "1Ul", "1UL", "1lu", "1lU", "1Lu", "1LU"] {
            assert_eq!(as_int(&single_clean(src)).1, IntSuffix::UL, "`{src}`");
        }
    }

    #[test]
    fn suffix_ll_matching_case_only() {
        assert_eq!(as_int(&single_clean("1ll")).1, IntSuffix::LL);
        assert_eq!(as_int(&single_clean("1LL")).1, IntSuffix::LL);
    }

    #[test]
    fn suffix_ll_mixed_case_is_not_ll() {
        // `1lL` → `1l` (L suffix) then identifier `L`.
        let (toks, diags) = lex_with_diags("1lL");
        assert!(diags.is_empty());
        assert_eq!(toks.len(), 2);
        let (v, s) = as_int(&toks[0].kind);
        assert_eq!(v, 1);
        assert_eq!(s, IntSuffix::L);
        assert_eq!(toks[1].kind, TokenKind::Identifier("L".to_string()));
    }

    #[test]
    fn suffix_ull_every_order() {
        for src in [
            "1ull", "1uLL", "1Ull", "1ULL", "1llu", "1llU", "1LLu", "1LLU",
        ] {
            assert_eq!(as_int(&single_clean(src)).1, IntSuffix::ULL, "`{src}`");
        }
    }

    #[test]
    fn hex_with_suffix() {
        let (v, s) = as_int(&single_clean("0xFFull"));
        assert_eq!(v, 0xFF);
        assert_eq!(s, IntSuffix::ULL);
    }

    #[test]
    fn octal_with_suffix() {
        let (v, s) = as_int(&single_clean("0777L"));
        assert_eq!(v, 0o777);
        assert_eq!(s, IntSuffix::L);
    }

    // =====================================================================
    // Decimal floats
    // =====================================================================

    #[test]
    fn simple_decimal_float() {
        let (v, s) = as_float(&single_clean("1.5"));
        assert!((v - 1.5).abs() < 1e-12);
        assert_eq!(s, FloatSuffix::None);
    }

    #[test]
    fn trailing_dot_float() {
        // "1." is a valid float (value 1.0) — the task explicitly lists
        // it as a legal form.
        let (v, _) = as_float(&single_clean("1."));
        assert!((v - 1.0).abs() < 1e-12);
    }

    #[test]
    fn leading_dot_float() {
        let (v, _) = as_float(&single_clean(".5"));
        assert!((v - 0.5).abs() < 1e-12);
    }

    #[test]
    fn float_with_positive_exponent() {
        let (v, _) = as_float(&single_clean("1e10"));
        assert!((v - 1e10).abs() < 1.0);
    }

    #[test]
    fn float_with_negative_exponent() {
        let (v, _) = as_float(&single_clean("1.5e-3"));
        assert!((v - 1.5e-3).abs() < 1e-15);
    }

    #[test]
    fn float_with_plus_exponent() {
        let (v, _) = as_float(&single_clean("2.5E+2"));
        assert!((v - 250.0).abs() < 1e-9);
    }

    #[test]
    fn float_dot_then_exponent() {
        // `1.e5` is a decimal float — 100000.0.
        let (v, _) = as_float(&single_clean("1.e5"));
        assert!((v - 1e5).abs() < 1.0);
    }

    #[test]
    fn float_dotless_exponent_only() {
        let (v, _) = as_float(&single_clean("3E4"));
        assert!((v - 3e4).abs() < 1.0);
    }

    #[test]
    fn float_exponent_without_digits_is_error() {
        let (_, diags) = lex_with_diags("1e");
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("exponent has no digits")),
            "expected exponent diagnostic, got {diags:?}"
        );
    }

    // ---------- Float suffixes ----------

    #[test]
    fn float_suffix_f() {
        let (_, s) = as_float(&single_clean("1.5f"));
        assert_eq!(s, FloatSuffix::F);
        let (_, s) = as_float(&single_clean("1.5F"));
        assert_eq!(s, FloatSuffix::F);
    }

    #[test]
    fn float_suffix_l() {
        let (_, s) = as_float(&single_clean("1.5l"));
        assert_eq!(s, FloatSuffix::L);
        let (_, s) = as_float(&single_clean("1.5L"));
        assert_eq!(s, FloatSuffix::L);
    }

    #[test]
    fn float_suffix_on_leading_dot() {
        let (v, s) = as_float(&single_clean(".25f"));
        assert!((v - 0.25).abs() < 1e-12);
        assert_eq!(s, FloatSuffix::F);
    }

    #[test]
    fn float_suffix_on_exponent_form() {
        let (v, s) = as_float(&single_clean("1e2F"));
        assert!((v - 100.0).abs() < 1e-9);
        assert_eq!(s, FloatSuffix::F);
    }

    // =====================================================================
    // Hex floats
    // =====================================================================

    #[test]
    fn hex_float_simple() {
        // 0x1.8p1 = (1 + 8/16) * 2^1 = 1.5 * 2 = 3.0
        let (v, _) = as_float(&single_clean("0x1.8p1"));
        assert!((v - 3.0).abs() < 1e-12);
    }

    #[test]
    fn hex_float_no_fractional_part() {
        // 0x1p3 = 1 * 8 = 8.0
        let (v, _) = as_float(&single_clean("0x1p3"));
        assert!((v - 8.0).abs() < 1e-12);
    }

    #[test]
    fn hex_float_no_integer_part() {
        // 0x.8p2 = 0.5 * 4 = 2.0
        let (v, _) = as_float(&single_clean("0x.8p2"));
        assert!((v - 2.0).abs() < 1e-12);
    }

    #[test]
    fn hex_float_negative_exponent() {
        // 0x1p-1 = 1 * 0.5 = 0.5
        let (v, _) = as_float(&single_clean("0x1p-1"));
        assert!((v - 0.5).abs() < 1e-12);
    }

    #[test]
    fn hex_float_uppercase_p() {
        let (v, _) = as_float(&single_clean("0x1P3"));
        assert!((v - 8.0).abs() < 1e-12);
    }

    #[test]
    fn hex_float_with_suffix() {
        let (v, s) = as_float(&single_clean("0x1.8p1f"));
        assert!((v - 3.0).abs() < 1e-12);
        assert_eq!(s, FloatSuffix::F);
    }

    #[test]
    fn hex_float_missing_binary_exponent_is_error() {
        let (_, diags) = lex_with_diags("0x1.5");
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("missing binary exponent")),
            "expected binary-exponent diagnostic, got {diags:?}"
        );
    }

    #[test]
    fn hex_float_p_without_digits_is_error() {
        let (_, diags) = lex_with_diags("0x1.5p");
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("exponent has no digits")),
            "expected exponent-no-digits diagnostic, got {diags:?}"
        );
    }

    // =====================================================================
    // Edge cases
    // =====================================================================

    #[test]
    fn dot_alone_is_dot_punctuator() {
        // Just `.` (no digit after) is the Dot punctuator.
        let toks = Lexer::new(".").tokenize();
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].kind, TokenKind::Dot);
    }

    #[test]
    fn ellipsis_is_still_ellipsis() {
        let toks = Lexer::new("...").tokenize();
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].kind, TokenKind::Ellipsis);
    }

    #[test]
    fn dot_followed_by_identifier_is_dot_then_identifier() {
        // `.x` — dot followed by identifier start, NOT a float.
        let (toks, diags) = lex_with_diags(".x");
        assert!(diags.is_empty());
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].kind, TokenKind::Dot);
        assert_eq!(toks[1].kind, TokenKind::Identifier("x".to_string()));
    }

    #[test]
    fn int_dot_method_splits_correctly() {
        // `1.method` → IntegerLiteral(1), Dot, Identifier("method").
        let (toks, diags) = lex_with_diags("1.method");
        assert!(diags.is_empty(), "unexpected diags: {diags:?}");
        assert_eq!(toks.len(), 3);
        let (v, _) = as_int(&toks[0].kind);
        assert_eq!(v, 1);
        assert_eq!(toks[1].kind, TokenKind::Dot);
        assert_eq!(toks[2].kind, TokenKind::Identifier("method".to_string()));
    }

    #[test]
    fn int_dot_underscore_splits_correctly() {
        // `1._x` — underscore starts an identifier.
        let (toks, _) = lex_with_diags("1._x");
        assert_eq!(toks.len(), 3);
        assert!(matches!(
            toks[0].kind,
            TokenKind::IntegerLiteral { value: 1, .. }
        ));
        assert_eq!(toks[1].kind, TokenKind::Dot);
        assert_eq!(toks[2].kind, TokenKind::Identifier("_x".to_string()));
    }

    #[test]
    fn int_then_dot_at_eof_is_float() {
        // `1.` alone — treated as float 1.0 per task spec.
        let (toks, diags) = lex_with_diags("1.");
        assert!(diags.is_empty());
        assert_eq!(toks.len(), 1);
        let (v, _) = as_float(&toks[0].kind);
        assert!((v - 1.0).abs() < 1e-12);
    }

    #[test]
    fn double_dot_after_integer_is_float_then_dot() {
        // `1..` — `1.` is a float (no identifier after dot), then `.`.
        let (toks, diags) = lex_with_diags("1..");
        assert!(diags.is_empty());
        assert_eq!(toks.len(), 2);
        let (v, _) = as_float(&toks[0].kind);
        assert!((v - 1.0).abs() < 1e-12);
        assert_eq!(toks[1].kind, TokenKind::Dot);
    }

    #[test]
    fn int_then_semicolon() {
        // Common case: sanity.
        let (toks, _) = lex_with_diags("42;");
        assert_eq!(toks.len(), 2);
        assert!(matches!(
            toks[0].kind,
            TokenKind::IntegerLiteral { value: 42, .. }
        ));
        assert_eq!(toks[1].kind, TokenKind::Semicolon);
    }

    #[test]
    fn float_then_semicolon() {
        let (toks, _) = lex_with_diags("1.5;");
        assert_eq!(toks.len(), 2);
        assert!(matches!(toks[0].kind, TokenKind::FloatLiteral { .. }));
        assert_eq!(toks[1].kind, TokenKind::Semicolon);
    }

    // =====================================================================
    // Spans
    // =====================================================================

    #[test]
    fn integer_literal_span_covers_value_and_suffix() {
        let (toks, _) = lex_with_diags("0xFFull");
        assert_eq!(toks[0].span, Span::new(0, 7));
    }

    #[test]
    fn float_literal_span_covers_value_and_suffix() {
        let (toks, _) = lex_with_diags("1.5e-3f");
        assert_eq!(toks[0].span, Span::new(0, 7));
    }

    #[test]
    fn float_from_leading_dot_span() {
        let (toks, _) = lex_with_diags(".5");
        assert_eq!(toks[0].span, Span::new(0, 2));
    }

    // =====================================================================
    // Mixed sequences — basic sanity that numeric literals cooperate
    // with the rest of the lexer.
    // =====================================================================

    #[test]
    fn expression_with_numbers_and_operators() {
        // `x = 3 + 4 * 0x10;`
        let (toks, diags) = lex_with_diags("x = 3 + 4 * 0x10;");
        assert!(diags.is_empty());
        let kinds: Vec<TokenKind> = toks.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Identifier("x".to_string()),
                TokenKind::Equal,
                TokenKind::IntegerLiteral {
                    value: 3,
                    suffix: IntSuffix::None
                },
                TokenKind::Plus,
                TokenKind::IntegerLiteral {
                    value: 4,
                    suffix: IntSuffix::None
                },
                TokenKind::Star,
                TokenKind::IntegerLiteral {
                    value: 16,
                    suffix: IntSuffix::None
                },
                TokenKind::Semicolon,
            ]
        );
    }

    #[test]
    fn return_zero_snippet() {
        // `return 0;` — makes sure the trivial case lexes right.
        let (toks, _) = lex_with_diags("return 0;");
        let kinds: Vec<TokenKind> = toks.into_iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Return,
                TokenKind::IntegerLiteral {
                    value: 0,
                    suffix: IntSuffix::None
                },
                TokenKind::Semicolon,
            ]
        );
    }

    // =====================================================================
    // Extra edge cases flagged while closing out phase 1.2
    // =====================================================================

    #[test]
    fn zero_with_every_integer_suffix() {
        // The decimal-zero path must honour suffixes just like non-zero.
        let cases: &[(&str, IntSuffix)] = &[
            ("0", IntSuffix::None),
            ("0u", IntSuffix::U),
            ("0U", IntSuffix::U),
            ("0l", IntSuffix::L),
            ("0L", IntSuffix::L),
            ("0ul", IntSuffix::UL),
            ("0UL", IntSuffix::UL),
            ("0lu", IntSuffix::UL),
            ("0ll", IntSuffix::LL),
            ("0LL", IntSuffix::LL),
            ("0ull", IntSuffix::ULL),
            ("0llu", IntSuffix::ULL),
            ("0LLU", IntSuffix::ULL),
        ];
        for (src, expected) in cases {
            let (v, s) = as_int(&single_clean(src));
            assert_eq!(v, 0, "`{src}` value");
            assert_eq!(s, *expected, "`{src}` suffix");
        }
    }

    #[test]
    fn hex_int_ending_with_f_is_hex_digit_not_float_suffix() {
        // `0x1f` is the hex integer 31 — `f` is a hex digit here, not the
        // `f` float suffix.  This disambiguation falls out of the greedy
        // hex-digit run plus the fact that `f` is only a float suffix on
        // an actual float literal.
        let (v, s) = as_int(&single_clean("0x1f"));
        assert_eq!(v, 0x1f);
        assert_eq!(s, IntSuffix::None);
    }

    #[test]
    fn hex_int_ending_with_l_is_hex_digit_run_then_l_suffix() {
        // `0x1L` — `L` is NOT a hex digit, so the hex-digit run ends at
        // `1` and `L` becomes the integer suffix.
        let (v, s) = as_int(&single_clean("0x1L"));
        assert_eq!(v, 1);
        assert_eq!(s, IntSuffix::L);
    }

    #[test]
    fn hex_float_with_l_suffix() {
        // Parallel to `hex_float_with_suffix` but exercising the `l` path.
        let (v, s) = as_float(&single_clean("0x1.8p1L"));
        assert!((v - 3.0).abs() < 1e-12);
        assert_eq!(s, FloatSuffix::L);
    }

    #[test]
    fn huge_decimal_exponent_becomes_infinity() {
        // f64 maxes out around 1e308; `1e9999` overflows to +inf.
        // This path exercises f64::parse() returning Ok(inf) — not an
        // error — so no diagnostic is emitted.  The lexer faithfully
        // hands the infinite value to later phases.
        let (toks, diags) = lex_with_diags("1e9999");
        assert!(diags.is_empty(), "no diagnostic expected: {diags:?}");
        assert_eq!(toks.len(), 1);
        let (v, _) = as_float(&toks[0].kind);
        assert!(v.is_infinite() && v.is_sign_positive());
    }

    #[test]
    fn decimal_then_hex_in_same_source() {
        // Sanity: numeric literals don't bleed across a whitespace boundary.
        let (toks, diags) = lex_with_diags("42 0xFF");
        assert!(diags.is_empty());
        assert_eq!(toks.len(), 2);
        let (a, _) = as_int(&toks[0].kind);
        let (b, _) = as_int(&toks[1].kind);
        assert_eq!(a, 42);
        assert_eq!(b, 0xFF);
    }

    #[test]
    fn hex_float_leading_dot_with_empty_integer_part() {
        // `0x.` (no hex digits on either side of the dot) emits the
        // "no hex digits" error and recovers as a 0.0 float.
        let (toks, diags) = lex_with_diags("0x.p0");
        assert!(
            diags.iter().any(|d| d.message.contains("no hex digits")),
            "expected `no hex digits` diagnostic, got {diags:?}"
        );
        // We still produce a float token so downstream phases don't choke.
        assert_eq!(toks.len(), 1);
        assert!(matches!(toks[0].kind, TokenKind::FloatLiteral { .. }));
    }

    #[test]
    fn int_run_followed_by_invalid_suffix_lexes_as_int_plus_ident() {
        // `1abc` is an integer `1` followed by identifier `abc`, not an
        // error.  (gcc emits an error in strict mode but the *lexer*
        // accepts it; a later phase may enforce semantics.)
        let (toks, diags) = lex_with_diags("1abc");
        assert!(diags.is_empty());
        assert_eq!(toks.len(), 2);
        let (v, s) = as_int(&toks[0].kind);
        assert_eq!(v, 1);
        assert_eq!(s, IntSuffix::None);
        assert_eq!(toks[1].kind, TokenKind::Identifier("abc".to_string()));
    }

    #[test]
    fn long_octal_exact_value() {
        // `0755` is the classic chmod bitset: 0o755 == 493.
        let (v, _) = as_int(&single_clean("0755"));
        assert_eq!(v, 0o755);
    }

    #[test]
    fn octal_u64_max_boundary() {
        // 0o1777777777777777777777 == 2^64 - 1.
        let (v, _) = as_int(&single_clean("01777777777777777777777"));
        assert_eq!(v, u64::MAX);
    }

    #[test]
    fn octal_overflow_emits_warning() {
        // 2^64 in octal needs one more digit.
        let (_, diags) = lex_with_diags("02000000000000000000000");
        assert!(
            diags.iter().any(|d| d.message.contains("too large")),
            "expected overflow warning, got {diags:?}"
        );
    }

    #[test]
    fn zero_point_zero_is_float_not_octal() {
        // `0.0` must route through the float path (dot + digit after zero
        // trumps octal interpretation).
        let (v, s) = as_float(&single_clean("0.0"));
        assert_eq!(v, 0.0);
        assert_eq!(s, FloatSuffix::None);
    }

    #[test]
    fn zero_exponent_is_float_not_octal() {
        // `0e5` must route through the float path (exponent trumps octal).
        let (v, _) = as_float(&single_clean("0e5"));
        assert_eq!(v, 0.0);
    }

    #[test]
    fn hex_literal_followed_by_dot_identifier_eats_dot() {
        // The documented trade-off for hex: `0x1.method` will be treated as
        // the start of a (malformed) hex float, NOT hex 0x1 + `.method`.
        // This test pins that behaviour so we notice if it ever changes.
        let (toks, diags) = lex_with_diags("0x1.method");
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("missing binary exponent")),
            "expected missing-exponent diagnostic, got {diags:?}"
        );
        // First token is a recovered hex float; then the `method` identifier.
        assert!(matches!(toks[0].kind, TokenKind::FloatLiteral { .. }));
        assert_eq!(toks[1].kind, TokenKind::Identifier("method".to_string()));
    }

    #[test]
    fn double_suffix_only_consumes_valid_tail() {
        // `1ulL` — `ul` matches the UL pattern, the trailing `L` is an
        // identifier.  This pins the longest-match table ordering so a
        // reordering bug can't regress silently.
        let (toks, diags) = lex_with_diags("1ulL");
        assert!(diags.is_empty());
        assert_eq!(toks.len(), 2);
        let (v, s) = as_int(&toks[0].kind);
        assert_eq!(v, 1);
        assert_eq!(s, IntSuffix::UL);
        assert_eq!(toks[1].kind, TokenKind::Identifier("L".to_string()));
    }
}
