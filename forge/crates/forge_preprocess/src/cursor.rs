//! Token cursor used by the preprocessor.
//!
//! The preprocessor walks its input from left to right but also needs to
//! **inject** tokens back into the stream — both when `#include`ing another
//! file and when a macro expands to a replacement list that must itself be
//! rescanned for further macro invocations.
//!
//! Internally [`TokenCursor`] holds its remaining tokens as a stack (a
//! reversed [`Vec`]): `peek` is the last element, `advance` is `pop`, and
//! `push_front` extends the stack with the injected tokens in reverse order
//! so that the first injected token is the next one consumed.  This keeps
//! every operation *O(1) amortised* and avoids the cost that a naive
//! `Vec::remove(0)` or `Vec::insert(0, _)` would incur.
//!
//! # Token type
//!
//! The cursor holds [`PPToken`]s — lexer tokens augmented with a
//! preprocessor *hide set* (see [`crate::pp_token`]).  Callers feeding a
//! raw lexer stream should first wrap it with [`crate::wrap_tokens`].
//!
//! # End of line
//!
//! Preprocessing directives are line-oriented: the argument list of a
//! directive ends at the first token whose `at_start_of_line` flag is
//! `true` (or at EOF).  Two helper methods express this explicitly:
//! [`TokenCursor::skip_to_end_of_line`] discards those tokens,
//! [`TokenCursor::collect_to_end_of_line`] returns them.  Both stop
//! **before** the end-of-line token so the main loop can re-enter with the
//! next logical line intact.

use forge_lexer::TokenKind;

use crate::pp_token::PPToken;

/// A cursor over a stream of preprocessor [`PPToken`]s.
///
/// See the module-level documentation for the overall design.  The cursor
/// owns its tokens and is consumed by [`TokenCursor::advance`]; finished
/// work leaves the stack empty.
#[derive(Debug, Clone)]
pub struct TokenCursor {
    /// Remaining tokens in **reverse** order.  The back of this vector is
    /// the next token returned by [`peek`](Self::peek) /
    /// [`advance`](Self::advance), so popping is O(1).
    stack: Vec<PPToken>,
}

impl TokenCursor {
    /// Build a cursor from an ordered [`PPToken`] stream.
    ///
    /// The first element of `tokens` will be the first one returned by
    /// [`peek`](Self::peek) / [`advance`](Self::advance).  Callers with a
    /// raw lexer [`Vec<Token>`](forge_lexer::Token) should wrap it with
    /// [`crate::wrap_tokens`] first.
    pub fn new(mut tokens: Vec<PPToken>) -> Self {
        tokens.reverse();
        Self { stack: tokens }
    }

    /// Peek at the next token without consuming it.
    ///
    /// Returns `None` when the cursor is exhausted.  Note that a lexer
    /// stream normally includes a trailing [`TokenKind::Eof`], so callers
    /// typically match on that kind to detect end-of-input rather than a
    /// `None` peek.
    pub fn peek(&self) -> Option<&PPToken> {
        self.stack.last()
    }

    /// Peek at the `n`-th upcoming token (0 = next).
    ///
    /// Useful for two-token look-ahead such as deciding between an
    /// object-like and a function-like macro definition (the `(` must
    /// immediately follow the macro name with no leading whitespace).
    pub fn peek_nth(&self, n: usize) -> Option<&PPToken> {
        if n >= self.stack.len() {
            return None;
        }
        Some(&self.stack[self.stack.len() - 1 - n])
    }

    /// Consume and return the next token.
    ///
    /// Returns `None` when the cursor is exhausted.
    pub fn advance(&mut self) -> Option<PPToken> {
        self.stack.pop()
    }

    /// Inject `tokens` at the front of the cursor so that the first element
    /// of `tokens` becomes the next one returned by
    /// [`peek`](Self::peek) / [`advance`](Self::advance).
    ///
    /// This is the primitive both `#include` (splice another file's tokens
    /// in) and macro expansion (splice the replacement list in for
    /// rescanning) use.
    pub fn push_front(&mut self, tokens: Vec<PPToken>) {
        self.stack.extend(tokens.into_iter().rev());
    }

    /// Discard tokens until the next token begins a new logical line
    /// (i.e. has `at_start_of_line == true`) or the stream ends.
    ///
    /// The sentinel token is **not** consumed — the cursor stops just
    /// before it so the caller can continue reading the next directive.
    /// A [`TokenKind::Eof`] is always treated as end-of-line.
    pub fn skip_to_end_of_line(&mut self) {
        while let Some(tok) = self.stack.pop() {
            if is_end_of_line(&tok) {
                self.stack.push(tok);
                return;
            }
        }
    }

    /// Collect and return tokens until end-of-line, stopping just before
    /// the next `at_start_of_line` token (or EOF).
    ///
    /// Tokens retain their original order: the first element of the
    /// returned vector is the one that was next in the stream.
    pub fn collect_to_end_of_line(&mut self) -> Vec<PPToken> {
        let mut out = Vec::new();
        while let Some(tok) = self.stack.pop() {
            if is_end_of_line(&tok) {
                self.stack.push(tok);
                break;
            }
            out.push(tok);
        }
        out
    }

    /// Number of tokens still in the cursor (including any trailing
    /// [`TokenKind::Eof`]).
    pub fn remaining(&self) -> usize {
        self.stack.len()
    }

    /// `true` when the cursor has no more tokens.
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

/// Is `tok` a logical end-of-line (start-of-next-line or EOF)?
fn is_end_of_line(tok: &PPToken) -> bool {
    tok.at_start_of_line() || matches!(tok.kind(), TokenKind::Eof)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pp_token::wrap_tokens;
    use forge_lexer::{Lexer, Token};

    fn lex(src: &str) -> Vec<Token> {
        Lexer::new(src).tokenize()
    }

    fn cur(src: &str) -> TokenCursor {
        TokenCursor::new(wrap_tokens(lex(src)))
    }

    fn kinds(toks: &[PPToken]) -> Vec<TokenKind> {
        toks.iter().map(|t| t.token.kind.clone()).collect()
    }

    #[test]
    fn peek_and_advance_walk_the_stream_in_order() {
        let mut cur = cur("a b c");
        assert!(matches!(
            cur.peek().map(|t| t.kind()),
            Some(TokenKind::Identifier(s)) if s == "a"
        ));
        let a = cur.advance().unwrap();
        assert!(matches!(a.token.kind, TokenKind::Identifier(ref s) if s == "a"));
        let b = cur.advance().unwrap();
        assert!(matches!(b.token.kind, TokenKind::Identifier(ref s) if s == "b"));
        let c = cur.advance().unwrap();
        assert!(matches!(c.token.kind, TokenKind::Identifier(ref s) if s == "c"));
        let eof = cur.advance().unwrap();
        assert!(matches!(eof.token.kind, TokenKind::Eof));
        assert!(cur.advance().is_none());
    }

    #[test]
    fn peek_nth_looks_ahead_without_consuming() {
        let cur = cur("a b c");
        assert!(
            matches!(cur.peek_nth(0), Some(t) if matches!(t.token.kind, TokenKind::Identifier(ref s) if s == "a"))
        );
        assert!(
            matches!(cur.peek_nth(1), Some(t) if matches!(t.token.kind, TokenKind::Identifier(ref s) if s == "b"))
        );
        assert!(
            matches!(cur.peek_nth(2), Some(t) if matches!(t.token.kind, TokenKind::Identifier(ref s) if s == "c"))
        );
        assert!(matches!(cur.peek_nth(3), Some(t) if matches!(t.token.kind, TokenKind::Eof)));
        assert!(cur.peek_nth(4).is_none());
    }

    #[test]
    fn push_front_injects_tokens_ahead_of_current_position() {
        let mut cur = cur("a c");
        // Inject `b` between `a` and `c`.
        let a = cur.advance().unwrap();
        assert!(matches!(a.token.kind, TokenKind::Identifier(ref s) if s == "a"));
        // Wrap a single `b` (drop the trailing Eof that the lexer tacks on).
        let injected: Vec<PPToken> = wrap_tokens(lex("b").into_iter().take(1));
        cur.push_front(injected);
        let b = cur.advance().unwrap();
        assert!(matches!(b.token.kind, TokenKind::Identifier(ref s) if s == "b"));
        let c = cur.advance().unwrap();
        assert!(matches!(c.token.kind, TokenKind::Identifier(ref s) if s == "c"));
    }

    #[test]
    fn push_front_preserves_order_of_injected_tokens() {
        let mut cur = cur("z");
        let injected: Vec<PPToken> = wrap_tokens(lex("x y").into_iter().take(2));
        cur.push_front(injected);
        let x = cur.advance().unwrap();
        assert!(matches!(x.token.kind, TokenKind::Identifier(ref s) if s == "x"));
        let y = cur.advance().unwrap();
        assert!(matches!(y.token.kind, TokenKind::Identifier(ref s) if s == "y"));
        let z = cur.advance().unwrap();
        assert!(matches!(z.token.kind, TokenKind::Identifier(ref s) if s == "z"));
    }

    #[test]
    fn skip_to_end_of_line_stops_before_sol_token() {
        let src = "a b\nc d";
        let mut cur = cur(src);
        cur.advance(); // consume `a`
        cur.skip_to_end_of_line();
        // Next token must be `c` (start of a new line).
        let next = cur.peek().unwrap();
        assert!(next.at_start_of_line());
        assert!(matches!(next.token.kind, TokenKind::Identifier(ref s) if s == "c"));
    }

    #[test]
    fn skip_to_end_of_line_stops_at_eof() {
        let mut cur = cur("a b c");
        cur.advance(); // `a`
        cur.skip_to_end_of_line();
        assert!(matches!(cur.peek().unwrap().token.kind, TokenKind::Eof));
    }

    #[test]
    fn collect_to_end_of_line_returns_the_line_tokens_in_order() {
        let src = "# define FOO 42\nint x;";
        let mut cur = cur(src);
        // Skip past `#` and `define`.
        cur.advance();
        cur.advance();
        let rest = cur.collect_to_end_of_line();
        let ks = kinds(&rest);
        // Expected: `FOO`, `42`
        assert!(matches!(ks[0], TokenKind::Identifier(ref s) if s == "FOO"));
        assert!(matches!(ks[1], TokenKind::IntegerLiteral { value: 42, .. }));
        assert_eq!(ks.len(), 2);
        // The next peek must be `int` on the new line.
        let next = cur.peek().unwrap();
        assert!(next.at_start_of_line());
        assert!(matches!(next.token.kind, TokenKind::Int));
    }

    #[test]
    fn collect_on_empty_line_returns_empty_vec() {
        let mut cur = cur("\nfoo");
        let got = cur.collect_to_end_of_line();
        assert!(got.is_empty());
        let next = cur.peek().unwrap();
        assert!(matches!(next.token.kind, TokenKind::Identifier(ref s) if s == "foo"));
    }

    #[test]
    fn remaining_and_is_empty_track_state() {
        let mut cur = cur("a b");
        let start = cur.remaining();
        assert!(start >= 2);
        assert!(!cur.is_empty());
        while cur.advance().is_some() {}
        assert!(cur.is_empty());
        assert_eq!(cur.remaining(), 0);
    }
}
