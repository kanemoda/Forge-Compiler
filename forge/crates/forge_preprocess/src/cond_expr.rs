//! Integer-constant-expression evaluator for `#if` and `#elif`.
//!
//! This module runs **after** the caller has already done two steps:
//!
//! 1. Replace `defined IDENT` and `defined(IDENT)` with the integer
//!    literal `1` (if defined) or `0` (if not).  This must happen *before*
//!    macro expansion so that `defined FOO` checks whether `FOO` is in
//!    the macro table rather than expanding it.
//! 2. Macro-expand every remaining token, then rewrite each surviving
//!    identifier to `0` per C17 §6.10.1/4.
//!
//! What lands here is therefore a pure sequence of integer/character
//! literals, punctuators, and `(` / `)` — exactly what a tiny Pratt
//! parser can turn into an [`PPValue`].
//!
//! # Signedness
//!
//! C17 §6.10.1/4 requires that `#if` constant expressions use `intmax_t`
//! / `uintmax_t`, applying the "usual arithmetic conversions" on every
//! binary operation — which in a two-type system reduce to "if either
//! operand is unsigned, both become unsigned".  This matters because
//! `#if -1 < 1U` is **false** in a real C preprocessor (the `-1` is
//! converted to `uintmax_t`, i.e. `UINTMAX_MAX`, which is not less than
//! 1).  A naïve `i64` evaluator gets that backwards.
//!
//! # Error recovery
//!
//! On a malformed expression the evaluator records an error diagnostic
//! and returns [`PPValue::Signed(0)`] so the surrounding `#if` is simply
//! treated as false.  Division by zero is a warning, not an error.

use forge_diagnostics::Diagnostic;
use forge_lexer::{IntSuffix, Span, Token, TokenKind};

use crate::expand::spelling_of;

/// A runtime value during `#if` expression evaluation — always integral,
/// tagged as either `intmax_t`- or `uintmax_t`-shaped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PPValue {
    /// A signed value (`intmax_t` in C parlance).
    Signed(i64),
    /// An unsigned value (`uintmax_t`).
    Unsigned(u64),
}

impl PPValue {
    /// The C boolean `0` — the representation the standard requires of a
    /// failed `#if` condition.
    pub const ZERO: Self = PPValue::Signed(0);

    /// The C boolean `1`.
    pub const ONE: Self = PPValue::Signed(1);

    /// Convenience: `1` if `b`, `0` otherwise, as a signed value.
    pub fn from_bool(b: bool) -> Self {
        if b {
            PPValue::ONE
        } else {
            PPValue::ZERO
        }
    }

    /// `true` iff the value equals `0` (either sign).
    pub fn is_zero(self) -> bool {
        match self {
            PPValue::Signed(n) => n == 0,
            PPValue::Unsigned(n) => n == 0,
        }
    }

    /// `true` iff the tag is [`PPValue::Unsigned`].
    pub fn is_unsigned(self) -> bool {
        matches!(self, PPValue::Unsigned(_))
    }

    /// Raw bit pattern as `u64`.  Signed values are reinterpreted via
    /// `as u64`, matching C's two's-complement conversion.
    pub fn as_u64(self) -> u64 {
        match self {
            PPValue::Signed(n) => n as u64,
            PPValue::Unsigned(n) => n,
        }
    }

    /// Raw bit pattern as `i64`.
    pub fn as_i64(self) -> i64 {
        match self {
            PPValue::Signed(n) => n,
            PPValue::Unsigned(n) => n as i64,
        }
    }
}

/// Evaluate the constant expression spelled out by `tokens`.
///
/// The tokens must already have gone through the two preparation steps
/// described in the module-level docs; `defined` and bare identifiers
/// are rejected here with an error diagnostic (they would indicate a
/// caller bug).
///
/// `if_location` is the span of the `#if` / `#elif` directive itself,
/// used as the fallback when a diagnostic needs a span and the parser is
/// at end-of-input.
pub fn evaluate(tokens: &[Token], if_location: Span) -> (PPValue, Vec<Diagnostic>) {
    let mut parser = Parser::new(tokens, if_location);
    let value = parser.parse_top();
    (value, parser.diagnostics)
}

// ---------------------------------------------------------------------------
// Internal: recursive-descent / Pratt parser
// ---------------------------------------------------------------------------

/// Binary operators recognised in an `#if` expression.
///
/// Deliberately not `pub` — callers interact with the evaluator through
/// [`evaluate`].
#[derive(Clone, Copy, Debug)]
enum BinOp {
    Mul,
    Div,
    Mod,
    Add,
    Sub,
    Shl,
    Shr,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    BitAnd,
    BitXor,
    BitOr,
    LogAnd,
    LogOr,
}

/// Pratt-parser binding powers for every binary operator.  Higher binds
/// tighter.  The left binding power is what controls "does this operator
/// steal the current subexpression?"; the right binding power controls
/// "how much does the RHS grab?".
///
/// All operators here are left-associative (`(l, l+1)`).  The ternary
/// `?:` is handled specially in [`Parser::parse_expr`] because it is
/// right-associative and has a three-operand shape.
const BP_TERNARY_LBP: u8 = 3;
const BP_TERNARY_RBP: u8 = 2;
const BP_UNARY_PREFIX_RBP: u8 = 25;

fn bp_binary(kind: &TokenKind) -> Option<(BinOp, u8, u8)> {
    use BinOp::*;
    Some(match kind {
        TokenKind::Star => (Mul, 22, 23),
        TokenKind::Slash => (Div, 22, 23),
        TokenKind::Percent => (Mod, 22, 23),
        TokenKind::Plus => (Add, 20, 21),
        TokenKind::Minus => (Sub, 20, 21),
        TokenKind::LessLess => (Shl, 18, 19),
        TokenKind::GreaterGreater => (Shr, 18, 19),
        TokenKind::Less => (Lt, 16, 17),
        TokenKind::Greater => (Gt, 16, 17),
        TokenKind::LessEqual => (Le, 16, 17),
        TokenKind::GreaterEqual => (Ge, 16, 17),
        TokenKind::EqualEqual => (Eq, 14, 15),
        TokenKind::BangEqual => (Ne, 14, 15),
        TokenKind::Ampersand => (BitAnd, 12, 13),
        TokenKind::Caret => (BitXor, 10, 11),
        TokenKind::Pipe => (BitOr, 8, 9),
        TokenKind::AmpAmp => (LogAnd, 6, 7),
        TokenKind::PipePipe => (LogOr, 4, 5),
        _ => return None,
    })
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    diagnostics: Vec<Diagnostic>,
    if_location: Span,
    /// Non-zero while parsing an un-taken short-circuit branch.  Runtime
    /// warnings (e.g. division-by-zero) are suppressed while this is
    /// positive, because per C17 §6.10.1 the value of a skipped branch
    /// is immaterial — only its syntactic shape matters.  Nesting is
    /// supported so `A && B && C` can short-circuit twice.
    suppress_runtime_warnings: u32,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token], if_location: Span) -> Self {
        Self {
            tokens,
            pos: 0,
            diagnostics: Vec::new(),
            if_location,
            suppress_runtime_warnings: 0,
        }
    }

    /// Parse the whole expression, emitting a diagnostic if any tokens
    /// remain after the top-level expression.
    fn parse_top(&mut self) -> PPValue {
        if self.at_end() {
            self.diagnostics
                .push(Diagnostic::error("empty `#if` expression").span(self.if_location.range()));
            return PPValue::ZERO;
        }
        let value = self.parse_expr(0);
        if !self.at_end() {
            let tok = &self.tokens[self.pos];
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "unexpected `{}` in `#if` expression",
                    spelling_of(&tok.kind)
                ))
                .span(tok.span.range()),
            );
        }
        value
    }

    fn parse_expr(&mut self, min_bp: u8) -> PPValue {
        let mut lhs = self.parse_prefix();

        loop {
            let op_tok = match self.peek() {
                Some(t) if !matches!(t.kind, TokenKind::Eof) => t,
                _ => break,
            };

            // Ternary first, since `?` is not a binary op in `bp_binary`.
            if matches!(op_tok.kind, TokenKind::Question) {
                if BP_TERNARY_LBP < min_bp {
                    break;
                }
                self.advance();
                let taken_is_mid = !lhs.is_zero();

                // Parse the middle operand, suppressing runtime warnings
                // when this branch will be discarded.  The middle of
                // `?:` is parsed from fresh — any top-level operator is
                // allowed inside.
                if !taken_is_mid {
                    self.suppress_runtime_warnings += 1;
                }
                let mid = self.parse_expr(0);
                if !taken_is_mid {
                    self.suppress_runtime_warnings -= 1;
                }

                match self.peek().map(|t| &t.kind) {
                    Some(TokenKind::Colon) => {
                        self.advance();
                    }
                    _ => {
                        let span = self.current_span();
                        self.diagnostics.push(
                            Diagnostic::error("expected `:` in conditional expression")
                                .span(span.range()),
                        );
                        return PPValue::ZERO;
                    }
                }

                if taken_is_mid {
                    self.suppress_runtime_warnings += 1;
                }
                let rhs = self.parse_expr(BP_TERNARY_RBP);
                if taken_is_mid {
                    self.suppress_runtime_warnings -= 1;
                }

                lhs = if taken_is_mid { mid } else { rhs };
                continue;
            }

            let (op, l_bp, r_bp) = match bp_binary(&op_tok.kind) {
                Some(v) => v,
                None => break,
            };
            if l_bp < min_bp {
                break;
            }
            let op_span = op_tok.span;
            self.advance();

            // Short-circuit evaluation for `&&` and `||`: the un-taken
            // branch is still parsed (to advance the cursor over its
            // tokens), but `suppress_runtime_warnings` silences any
            // division-by-zero or modulo-by-zero warning that would
            // otherwise fire.  Syntactic errors are preserved so a
            // malformed skipped branch is still reported.
            match op {
                BinOp::LogAnd => {
                    if lhs.is_zero() {
                        self.suppress_runtime_warnings += 1;
                        let _skipped = self.parse_expr(r_bp);
                        self.suppress_runtime_warnings -= 1;
                        lhs = PPValue::from_bool(false);
                    } else {
                        let rhs = self.parse_expr(r_bp);
                        lhs = PPValue::from_bool(!rhs.is_zero());
                    }
                    continue;
                }
                BinOp::LogOr => {
                    if !lhs.is_zero() {
                        self.suppress_runtime_warnings += 1;
                        let _skipped = self.parse_expr(r_bp);
                        self.suppress_runtime_warnings -= 1;
                        lhs = PPValue::from_bool(true);
                    } else {
                        let rhs = self.parse_expr(r_bp);
                        lhs = PPValue::from_bool(!rhs.is_zero());
                    }
                    continue;
                }
                _ => {}
            }

            let rhs = self.parse_expr(r_bp);
            lhs = self.apply_binop(lhs, op, rhs, op_span);
        }

        lhs
    }

    fn parse_prefix(&mut self) -> PPValue {
        let tok = match self.peek() {
            Some(t) => t.clone(),
            None => {
                self.diagnostics.push(
                    Diagnostic::error("expected expression in `#if`")
                        .span(self.if_location.range()),
                );
                return PPValue::ZERO;
            }
        };
        match &tok.kind {
            TokenKind::Eof => {
                self.diagnostics.push(
                    Diagnostic::error("expected expression in `#if`")
                        .span(self.if_location.range()),
                );
                PPValue::ZERO
            }
            TokenKind::IntegerLiteral { value, suffix } => {
                self.advance();
                if matches!(suffix, IntSuffix::U | IntSuffix::UL | IntSuffix::ULL) {
                    PPValue::Unsigned(*value)
                } else {
                    PPValue::Signed(*value as i64)
                }
            }
            TokenKind::CharLiteral { value, .. } => {
                self.advance();
                PPValue::Signed(*value as i64)
            }
            TokenKind::LeftParen => {
                self.advance();
                let v = self.parse_expr(0);
                match self.peek().map(|t| &t.kind) {
                    Some(TokenKind::RightParen) => {
                        self.advance();
                    }
                    _ => {
                        let span = self.current_span();
                        self.diagnostics.push(
                            Diagnostic::error("expected `)` in `#if` expression")
                                .span(span.range()),
                        );
                    }
                }
                v
            }
            TokenKind::Plus => {
                self.advance();
                self.parse_expr(BP_UNARY_PREFIX_RBP)
            }
            TokenKind::Minus => {
                self.advance();
                let v = self.parse_expr(BP_UNARY_PREFIX_RBP);
                match v {
                    PPValue::Signed(n) => PPValue::Signed(n.wrapping_neg()),
                    PPValue::Unsigned(n) => PPValue::Unsigned(0u64.wrapping_sub(n)),
                }
            }
            TokenKind::Bang => {
                self.advance();
                let v = self.parse_expr(BP_UNARY_PREFIX_RBP);
                PPValue::from_bool(v.is_zero())
            }
            TokenKind::Tilde => {
                self.advance();
                let v = self.parse_expr(BP_UNARY_PREFIX_RBP);
                match v {
                    PPValue::Signed(n) => PPValue::Signed(!n),
                    PPValue::Unsigned(n) => PPValue::Unsigned(!n),
                }
            }
            _ => {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "unexpected `{}` in `#if` expression",
                        spelling_of(&tok.kind)
                    ))
                    .span(tok.span.range()),
                );
                self.advance();
                PPValue::ZERO
            }
        }
    }

    fn apply_binop(&mut self, lhs: PPValue, op: BinOp, rhs: PPValue, span: Span) -> PPValue {
        use BinOp::*;

        // Comparisons: result type is always signed 0 / 1.  The
        // comparison itself is performed in the usual-arithmetic-
        // converted type.
        match op {
            Lt | Gt | Le | Ge | Eq | Ne => {
                let unsigned = lhs.is_unsigned() || rhs.is_unsigned();
                let result = if unsigned {
                    let l = lhs.as_u64();
                    let r = rhs.as_u64();
                    match op {
                        Lt => l < r,
                        Gt => l > r,
                        Le => l <= r,
                        Ge => l >= r,
                        Eq => l == r,
                        Ne => l != r,
                        _ => unreachable!(),
                    }
                } else {
                    let l = lhs.as_i64();
                    let r = rhs.as_i64();
                    match op {
                        Lt => l < r,
                        Gt => l > r,
                        Le => l <= r,
                        Ge => l >= r,
                        Eq => l == r,
                        Ne => l != r,
                        _ => unreachable!(),
                    }
                };
                return PPValue::from_bool(result);
            }
            _ => {}
        }

        // Shifts: the result type follows the left operand after
        // integer promotion — i.e. the left operand's signedness wins.
        // Shift amounts are masked to 0..64 to avoid Rust panics and
        // match GCC's /clang's shift behaviour on 64-bit.
        match op {
            Shl | Shr => {
                let count = (rhs.as_u64() & 63) as u32;
                return match lhs {
                    PPValue::Signed(n) => PPValue::Signed(match op {
                        Shl => (n as u64).wrapping_shl(count) as i64,
                        Shr => n.wrapping_shr(count),
                        _ => unreachable!(),
                    }),
                    PPValue::Unsigned(n) => PPValue::Unsigned(match op {
                        Shl => n.wrapping_shl(count),
                        Shr => n.wrapping_shr(count),
                        _ => unreachable!(),
                    }),
                };
            }
            _ => {}
        }

        // Arithmetic & bitwise: apply usual arithmetic conversions.
        let unsigned = lhs.is_unsigned() || rhs.is_unsigned();
        if unsigned {
            let l = lhs.as_u64();
            let r = rhs.as_u64();
            let val = match op {
                Mul => l.wrapping_mul(r),
                Div => {
                    if r == 0 {
                        self.warn_div_by_zero("division", span);
                        0
                    } else {
                        l.wrapping_div(r)
                    }
                }
                Mod => {
                    if r == 0 {
                        self.warn_div_by_zero("modulo", span);
                        0
                    } else {
                        l.wrapping_rem(r)
                    }
                }
                Add => l.wrapping_add(r),
                Sub => l.wrapping_sub(r),
                BitAnd => l & r,
                BitXor => l ^ r,
                BitOr => l | r,
                _ => unreachable!(),
            };
            PPValue::Unsigned(val)
        } else {
            let l = lhs.as_i64();
            let r = rhs.as_i64();
            let val = match op {
                Mul => l.wrapping_mul(r),
                Div => {
                    if r == 0 {
                        self.warn_div_by_zero("division", span);
                        0
                    } else {
                        l.wrapping_div(r)
                    }
                }
                Mod => {
                    if r == 0 {
                        self.warn_div_by_zero("modulo", span);
                        0
                    } else {
                        l.wrapping_rem(r)
                    }
                }
                Add => l.wrapping_add(r),
                Sub => l.wrapping_sub(r),
                BitAnd => l & r,
                BitXor => l ^ r,
                BitOr => l | r,
                _ => unreachable!(),
            };
            PPValue::Signed(val)
        }
    }

    fn warn_div_by_zero(&mut self, what: &str, span: Span) {
        if self.suppress_runtime_warnings > 0 {
            return;
        }
        self.diagnostics.push(
            Diagnostic::warning(format!("{what} by zero in `#if` expression")).span(span.range()),
        );
    }

    // --- cursor helpers ----------------------------------------------

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) {
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
    }

    fn at_end(&self) -> bool {
        match self.peek() {
            None => true,
            Some(t) => matches!(t.kind, TokenKind::Eof),
        }
    }

    fn current_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or(self.if_location)
    }
}
