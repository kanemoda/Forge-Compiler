//! Preprocessor token — a lexer [`Token`] augmented with a *hide set*.
//!
//! C17 §6.10.3.4 requires that every preprocessor token carries the set
//! of macro names whose expansion is currently suppressed for that
//! token.  This hide set is the mechanism that prevents infinite
//! recursion: once macro `M` has been expanded, every token in the
//! resulting replacement list carries `M` in its hide set, so a later
//! rescan cannot re-enter `M`'s definition.
//!
//! We keep the hide set on a wrapper type ([`PPToken`]) rather than on
//! `forge_lexer::Token` itself.  The lexer and downstream parser have
//! no reason to know about hide sets; the preprocessor is the only
//! pipeline phase that reads or writes them.  Keeping the concern
//! local here means the lexer's `Token` stays a plain POD.
//!
//! # Lifecycle
//!
//! * [`PPToken::new`] wraps a lexer token with an empty hide set — this
//!   is how tokens enter the preprocessor.
//! * During expansion, the hide set for a replacement token is computed
//!   as `invocation_token.hide_set ∪ { macro_name }` (see
//!   [`Preprocessor::run`](crate::Preprocessor::run)).
//! * [`PPToken::into_token`] strips the hide set on the way out to the
//!   parser.

use std::collections::HashSet;

use forge_lexer::{Span, Token, TokenKind};

/// A [`Token`] paired with the set of macro names whose expansion is
/// blocked for this token.
///
/// Most field access goes through the public [`PPToken::token`] field;
/// the methods here are convenience shorthands for the two most
/// frequently touched fields (`kind`, `span`) and the hide-set
/// manipulation that the expansion loop performs.
#[derive(Clone, Debug, PartialEq)]
pub struct PPToken {
    /// The underlying lexer token.
    pub token: Token,
    /// Macro names that may **not** be expanded from this token.
    ///
    /// `HashSet<String>` is chosen over a more compact representation
    /// (bitset keyed by macro id, interned string id, …) for clarity
    /// while the preprocessor is under active development; switching
    /// to interned ids later is a drop-in optimisation.
    pub hide_set: HashSet<String>,
}

impl PPToken {
    /// Wrap `token` with an empty hide set.
    ///
    /// This is how tokens are lifted into the preprocessor at the
    /// input boundary (and, during expansion, how newly-paste-generated
    /// tokens are introduced before their hide set is computed).
    pub fn new(token: Token) -> Self {
        Self {
            token,
            hide_set: HashSet::new(),
        }
    }

    /// Wrap `token` with the given hide set.
    pub fn with_hide_set(token: Token, hide_set: HashSet<String>) -> Self {
        Self { token, hide_set }
    }

    /// Borrow the inner token's kind.
    pub fn kind(&self) -> &TokenKind {
        &self.token.kind
    }

    /// The inner token's source span.
    pub fn span(&self) -> Span {
        self.token.span
    }

    /// `true` iff the inner token begins a new logical line.
    pub fn at_start_of_line(&self) -> bool {
        self.token.at_start_of_line
    }

    /// `true` iff the inner token was preceded by whitespace.
    pub fn has_leading_space(&self) -> bool {
        self.token.has_leading_space
    }

    /// `true` iff `macro_name` is in this token's hide set — i.e. the
    /// preprocessor must not expand it here.
    pub fn is_hidden(&self, macro_name: &str) -> bool {
        self.hide_set.contains(macro_name)
    }

    /// Strip the hide set and return the inner lexer token.
    ///
    /// Used when producing the preprocessor's final output stream,
    /// which downstream phases consume as plain [`Token`]s.
    pub fn into_token(self) -> Token {
        self.token
    }
}

/// Wrap a plain token stream (as produced by [`forge_lexer::Lexer`])
/// into [`PPToken`]s with empty hide sets.
pub fn wrap_tokens<I>(tokens: I) -> Vec<PPToken>
where
    I: IntoIterator<Item = Token>,
{
    tokens.into_iter().map(PPToken::new).collect()
}

/// Strip the hide sets from a preprocessor token stream, returning the
/// plain [`Token`]s that downstream phases consume.
pub fn unwrap_tokens<I>(pp_tokens: I) -> Vec<Token>
where
    I: IntoIterator<Item = PPToken>,
{
    pp_tokens.into_iter().map(PPToken::into_token).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use forge_lexer::Lexer;

    fn first_token(src: &str) -> Token {
        Lexer::new(src)
            .tokenize()
            .into_iter()
            .next()
            .expect("lexer always yields at least Eof")
    }

    #[test]
    fn new_creates_token_with_empty_hide_set() {
        let pp = PPToken::new(first_token("foo"));
        assert!(pp.hide_set.is_empty());
        assert!(matches!(pp.kind(), TokenKind::Identifier(s) if s == "foo"));
    }

    #[test]
    fn with_hide_set_preserves_the_supplied_set() {
        let mut set = HashSet::new();
        set.insert("FOO".to_string());
        set.insert("BAR".to_string());
        let pp = PPToken::with_hide_set(first_token("baz"), set);
        assert_eq!(pp.hide_set.len(), 2);
        assert!(pp.is_hidden("FOO"));
        assert!(pp.is_hidden("BAR"));
        assert!(!pp.is_hidden("QUX"));
    }

    #[test]
    fn accessor_shorthands_delegate_to_the_inner_token() {
        let pp = PPToken::new(first_token("int"));
        assert!(matches!(pp.kind(), TokenKind::Int));
        assert_eq!(pp.span().start, 0);
        // First token on the line → at_start_of_line is true.
        assert!(pp.at_start_of_line());
        assert!(!pp.has_leading_space());
    }

    #[test]
    fn into_token_strips_the_hide_set() {
        let mut set = HashSet::new();
        set.insert("M".to_string());
        let original = first_token("x");
        let pp = PPToken::with_hide_set(original.clone(), set);
        let back = pp.into_token();
        assert_eq!(back, original);
    }

    #[test]
    fn wrap_and_unwrap_round_trip_a_token_stream() {
        let tokens = Lexer::new("int x = 1;").tokenize();
        let wrapped = wrap_tokens(tokens.clone());
        assert_eq!(wrapped.len(), tokens.len());
        assert!(wrapped.iter().all(|p| p.hide_set.is_empty()));
        let unwrapped = unwrap_tokens(wrapped);
        assert_eq!(unwrapped, tokens);
    }

    #[test]
    fn unwrap_drops_the_hide_sets() {
        let mut set = HashSet::new();
        set.insert("ANY".to_string());
        let pp = PPToken::with_hide_set(first_token("x"), set);
        let plain = unwrap_tokens(vec![pp]);
        assert_eq!(plain.len(), 1);
        // The unwrapped token matches what the lexer produced; the hide
        // set is simply gone.
        assert!(matches!(plain[0].kind, TokenKind::Identifier(ref s) if s == "x"));
    }
}
