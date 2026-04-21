//! Parser infrastructure: token cursor, typedef scope, diagnostics, and
//! the top-level entry point.
//!
//! The parser is a hand-written recursive descent parser that operates
//! on the flat token stream produced by the lexer (after preprocessing).
//! Expression parsing uses a Pratt (top-down operator-precedence)
//! scheme in [`super::expr`].

use std::collections::HashSet;

use forge_diagnostics::Diagnostic;
use forge_lexer::{Span, Token, TokenKind};

use crate::ast::*;
use crate::node_id::NodeId;

/// Recursive descent parser for C17.
///
/// Constructed via [`Parser::new`] and driven via [`Parser::parse`].
/// After parsing, the caller inspects the returned diagnostics to
/// decide whether the AST is usable.
pub struct Parser {
    /// Flat token stream (including trailing `Eof`).
    tokens: Vec<Token>,
    /// Current position into `tokens`.
    pos: usize,
    /// Stack of typedef scopes.  The last entry is the current scope.
    /// An identifier is a typedef iff it appears in *any* scope (walk
    /// top-to-bottom), which implements C's scoping rules where an
    /// inner typedef shadows the outer one until the scope closes.
    typedefs: Vec<HashSet<String>>,
    /// Accumulated diagnostics.
    diagnostics: Vec<Diagnostic>,
    /// Set to `true` when any error-severity diagnostic is emitted.
    has_errors: bool,
    /// Monotonic counter handed out by [`Parser::next_id`].  Each value
    /// is used exactly once per parse of a translation unit.
    next_node_id: u32,
}

impl Parser {
    /// Create a new parser over the given token stream.
    ///
    /// The lexer always terminates its output with [`TokenKind::Eof`];
    /// to make this assumption bullet-proof against a caller that hands
    /// us an empty or non-terminated vector (and thus keep [`peek`] and
    /// [`peek_ahead`] panic-free), we append a synthetic `Eof` with a
    /// zero-width span at the end of the previous span when the
    /// invariant does not already hold.
    ///
    /// [`peek`]: Parser::peek
    /// [`peek_ahead`]: Parser::peek_ahead
    pub fn new(mut tokens: Vec<Token>) -> Self {
        if !matches!(tokens.last().map(|t| &t.kind), Some(TokenKind::Eof)) {
            let span = tokens.last().map_or(Span::primary(0, 0), |t| {
                Span::new(t.span.file, t.span.end, t.span.end)
            });
            tokens.push(Token {
                kind: TokenKind::Eof,
                span,
                at_start_of_line: true,
                has_leading_space: false,
            });
        }

        let mut initial_scope = HashSet::new();
        // GCC builtin typedef used heavily in system-header output.
        initial_scope.insert("__builtin_va_list".to_string());
        Self {
            tokens,
            pos: 0,
            typedefs: vec![initial_scope],
            diagnostics: Vec::new(),
            has_errors: false,
            next_node_id: 0,
        }
    }

    /// Mint a fresh [`NodeId`] and advance the internal counter.
    ///
    /// Every AST variant that semantic analysis will annotate obtains its
    /// id through this method.  IDs are dense, start at zero, and are
    /// unique within a single `Parser`'s lifetime — see
    /// [`crate::node_id`] for the cross-parse stability note.
    pub(crate) fn next_id(&mut self) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        NodeId(id)
    }

    /// Parse a complete translation unit.
    ///
    /// Always returns an AST (possibly partial) plus diagnostics.
    /// Callers inspect the diagnostics to decide whether errors occurred.
    pub fn parse(tokens: Vec<Token>) -> (TranslationUnit, Vec<Diagnostic>) {
        let mut parser = Self::new(tokens);
        let tu = parser.parse_translation_unit();
        (tu, parser.diagnostics)
    }

    // =====================================================================
    // Token access
    // =====================================================================

    /// Peek at the current token without consuming it.
    ///
    /// Reads past the end of the stream are clamped to the trailing
    /// [`TokenKind::Eof`] sentinel that [`Parser::new`] guarantees is
    /// present — callers never see `None`, so there is no panic path
    /// here regardless of how `pos` has been advanced.
    pub(crate) fn peek(&self) -> &Token {
        // SAFETY (soundness): `Parser::new` ensures `self.tokens`
        // always ends with an `Eof` token, so `last()` is `Some` and
        // the `.expect` is load-bearing documentation of an invariant
        // that cannot fail by construction.
        self.tokens.get(self.pos).unwrap_or_else(|| {
            self.tokens
                .last()
                .expect("invariant: Parser::new appends Eof when absent")
        })
    }

    /// Peek at the token `n` positions ahead (0 = current).
    ///
    /// Same clamping semantics as [`Parser::peek`]: reads past the end
    /// of the stream return the synthetic `Eof` sentinel rather than
    /// panicking.
    pub(crate) fn peek_ahead(&self, n: usize) -> &Token {
        // SAFETY (soundness): see the note on `Parser::peek`.
        self.tokens.get(self.pos + n).unwrap_or_else(|| {
            self.tokens
                .last()
                .expect("invariant: Parser::new appends Eof when absent")
        })
    }

    /// The current cursor position (absolute index into the token stream).
    pub(crate) fn cursor(&self) -> usize {
        self.pos
    }

    /// Look at the token at an absolute position, returning `None`
    /// past the end of the stream.
    pub(crate) fn token_at(&self, pos: usize) -> Option<&Token> {
        self.tokens.get(pos)
    }

    /// Consume and return the current token, advancing the cursor.
    pub(crate) fn advance(&mut self) -> Token {
        let tok = self.peek().clone();
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    /// Consume the current token if it matches `kind`, returning it.
    /// Returns `None` (and does not advance) if the token doesn't match.
    pub(crate) fn eat(&mut self, kind: &TokenKind) -> Option<Token> {
        if self.at(kind) {
            Some(self.advance())
        } else {
            None
        }
    }

    /// `true` if the current token matches `kind`.
    ///
    /// For variants with data (`Identifier`, `IntegerLiteral`, etc.)
    /// this uses [`std::mem::discriminant`] so the payload is ignored.
    pub(crate) fn at(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(kind)
    }

    /// `true` if the current token is `Eof`.
    pub(crate) fn at_eof(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    /// Consume the current token if it matches `kind`.  On mismatch
    /// emit a diagnostic and return `Err(())`.
    pub(crate) fn expect(&mut self, kind: &TokenKind) -> Result<Token, ()> {
        if self.at(kind) {
            Ok(self.advance())
        } else {
            let got = self.peek().clone();
            self.error(
                format!(
                    "expected `{}`, found `{}`",
                    kind_name(kind),
                    kind_name(&got.kind)
                ),
                got.span,
            );
            Err(())
        }
    }

    // =====================================================================
    // State save / restore (for backtracking)
    // =====================================================================

    /// Capture the full parser state (position + diagnostic bookkeeping).
    ///
    /// Returned value is consumed by [`Parser::restore_state`] to undo
    /// any side-effects — including diagnostics — accumulated during a
    /// speculative parse.
    pub(crate) fn save_state(&self) -> ParserState {
        ParserState {
            pos: self.pos,
            diagnostics_len: self.diagnostics.len(),
            has_errors: self.has_errors,
        }
    }

    /// Rewind to a previously saved state, dropping any diagnostics and
    /// error flags produced since.
    pub(crate) fn restore_state(&mut self, state: ParserState) {
        self.pos = state.pos;
        self.diagnostics.truncate(state.diagnostics_len);
        self.has_errors = state.has_errors;
    }

    // =====================================================================
    // Span helpers
    // =====================================================================

    /// Build a span from `start` to the end of the previously consumed
    /// token.  If nothing was consumed yet, returns `start` as-is.
    pub(crate) fn span_from(&self, start: Span) -> Span {
        if self.pos == 0 {
            return start;
        }
        let prev = &self.tokens[self.pos - 1];
        Span::new(start.file, start.start, prev.span.end)
    }

    // =====================================================================
    // Typedef scope
    // =====================================================================

    /// Push a new (empty) typedef scope.
    pub(crate) fn push_scope(&mut self) {
        self.typedefs.push(HashSet::new());
    }

    /// Pop the innermost typedef scope.
    pub(crate) fn pop_scope(&mut self) {
        if self.typedefs.len() > 1 {
            self.typedefs.pop();
        }
    }

    /// Register `name` as a typedef in the current (innermost) scope.
    pub(crate) fn add_typedef(&mut self, name: &str) {
        if let Some(scope) = self.typedefs.last_mut() {
            scope.insert(name.to_string());
        }
    }

    /// `true` if `name` is a typedef in any active scope.
    pub(crate) fn is_typedef(&self, name: &str) -> bool {
        self.typedefs.iter().rev().any(|scope| scope.contains(name))
    }

    // =====================================================================
    // Diagnostics
    // =====================================================================

    /// Record an error diagnostic.
    pub(crate) fn error(&mut self, message: impl Into<String>, span: Span) {
        self.has_errors = true;
        self.diagnostics.push(Diagnostic::error(message).span(span));
    }

    /// Record a warning diagnostic.
    #[allow(dead_code)]
    pub(crate) fn warning(&mut self, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::warning(message).span(span));
    }

    /// Drain and return all accumulated diagnostics, leaving the parser
    /// with an empty diagnostic buffer.  Used by tests to inspect errors.
    #[cfg(test)]
    pub(crate) fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        self.has_errors = false;
        std::mem::take(&mut self.diagnostics)
    }

    // =====================================================================
    // Type-name helpers
    // =====================================================================

    /// `true` if `kind` can start a type-name (specifier or qualifier).
    ///
    /// Used by the Pratt expression parser to resolve the cast /
    /// compound-literal / parenthesised-expression ambiguity, and by
    /// [`Parser::is_start_of_declaration`] for declaration lookahead.
    pub(crate) fn token_starts_type_name(&self, kind: &TokenKind) -> bool {
        matches!(
            kind,
            // Type specifier keywords
            TokenKind::Void
                | TokenKind::Char
                | TokenKind::Short
                | TokenKind::Int
                | TokenKind::Long
                | TokenKind::Float
                | TokenKind::Double
                | TokenKind::Signed
                | TokenKind::Unsigned
                | TokenKind::Bool
                | TokenKind::Complex
                | TokenKind::Atomic
                | TokenKind::Struct
                | TokenKind::Union
                | TokenKind::Enum
                // Type qualifiers
                | TokenKind::Const
                | TokenKind::Volatile
                | TokenKind::Restrict
        ) || self.is_typedef_token(kind)
            || is_gnu_type_keyword_token(kind)
    }

    /// `true` if `kind` is an `Identifier` that names a typedef.
    pub(crate) fn is_typedef_token(&self, kind: &TokenKind) -> bool {
        match kind {
            TokenKind::Identifier(name) => self.is_typedef(name),
            _ => false,
        }
    }
}

// =========================================================================
// Parser state snapshot
// =========================================================================

/// Snapshot of the parser's mutable state for speculative-parse rollback.
///
/// Returned by [`Parser::save_state`]; consumed by
/// [`Parser::restore_state`].  Includes the cursor position *and* the
/// diagnostics count so that any errors emitted during the speculation
/// are also discarded on rollback.
#[derive(Clone, Copy)]
pub(crate) struct ParserState {
    pos: usize,
    diagnostics_len: usize,
    has_errors: bool,
}

// =========================================================================
// GNU keyword recognition
// =========================================================================

/// `true` if `kind` is a GNU identifier keyword that can appear in a
/// declaration specifier position.
///
/// GNU keywords are lexed as `Identifier` because they are not part of
/// the C17 keyword set.  We recognize them in the declaration path only.
pub(crate) fn is_gnu_type_keyword_token(kind: &TokenKind) -> bool {
    match kind {
        TokenKind::Identifier(name) => is_gnu_type_keyword(name),
        _ => false,
    }
}

/// `true` if `s` is a GNU keyword that can appear in a declaration
/// specifier position (qualifier, type specifier, function specifier,
/// or attribute).
pub(crate) fn is_gnu_type_keyword(s: &str) -> bool {
    matches!(
        s,
        "__const"
            | "__const__"
            | "__volatile"
            | "__volatile__"
            | "__restrict"
            | "__restrict__"
            | "__inline"
            | "__inline__"
            | "__signed"
            | "__signed__"
            | "__extension__"
            | "__attribute__"
            | "__typeof__"
            | "__typeof"
            | "typeof"
            // Compiler-extension scalar type names that also appear as
            // type-specifiers (see `is_ext_type_name` in `decl.rs`).
            | "_Float16"
            | "_Float32"
            | "_Float64"
            | "_Float128"
            | "_Float32x"
            | "_Float64x"
            | "_Float128x"
            | "__int128"
            | "__int128_t"
            | "__uint128_t"
            | "_Decimal32"
            | "_Decimal64"
            | "_Decimal128"
    )
}

// =========================================================================
// Display helpers
// =========================================================================

/// Human-readable name for a token kind, used in diagnostics.
pub(crate) fn kind_name(kind: &TokenKind) -> &'static str {
    match kind {
        TokenKind::Auto => "auto",
        TokenKind::Break => "break",
        TokenKind::Case => "case",
        TokenKind::Char => "char",
        TokenKind::Const => "const",
        TokenKind::Continue => "continue",
        TokenKind::Default => "default",
        TokenKind::Do => "do",
        TokenKind::Double => "double",
        TokenKind::Else => "else",
        TokenKind::Enum => "enum",
        TokenKind::Extern => "extern",
        TokenKind::Float => "float",
        TokenKind::For => "for",
        TokenKind::Goto => "goto",
        TokenKind::If => "if",
        TokenKind::Inline => "inline",
        TokenKind::Int => "int",
        TokenKind::Long => "long",
        TokenKind::Register => "register",
        TokenKind::Restrict => "restrict",
        TokenKind::Return => "return",
        TokenKind::Short => "short",
        TokenKind::Signed => "signed",
        TokenKind::Sizeof => "sizeof",
        TokenKind::Static => "static",
        TokenKind::Struct => "struct",
        TokenKind::Switch => "switch",
        TokenKind::Typedef => "typedef",
        TokenKind::Union => "union",
        TokenKind::Unsigned => "unsigned",
        TokenKind::Void => "void",
        TokenKind::Volatile => "volatile",
        TokenKind::While => "while",
        TokenKind::Alignas => "_Alignas",
        TokenKind::Alignof => "_Alignof",
        TokenKind::Atomic => "_Atomic",
        TokenKind::Bool => "_Bool",
        TokenKind::Complex => "_Complex",
        TokenKind::Generic => "_Generic",
        TokenKind::Imaginary => "_Imaginary",
        TokenKind::Noreturn => "_Noreturn",
        TokenKind::StaticAssert => "_Static_assert",
        TokenKind::ThreadLocal => "_Thread_local",
        TokenKind::LeftParen => "(",
        TokenKind::RightParen => ")",
        TokenKind::LeftBrace => "{",
        TokenKind::RightBrace => "}",
        TokenKind::LeftBracket => "[",
        TokenKind::RightBracket => "]",
        TokenKind::Dot => ".",
        TokenKind::Arrow => "->",
        TokenKind::PlusPlus => "++",
        TokenKind::MinusMinus => "--",
        TokenKind::Ampersand => "&",
        TokenKind::Star => "*",
        TokenKind::Plus => "+",
        TokenKind::Minus => "-",
        TokenKind::Tilde => "~",
        TokenKind::Bang => "!",
        TokenKind::Slash => "/",
        TokenKind::Percent => "%",
        TokenKind::LessLess => "<<",
        TokenKind::GreaterGreater => ">>",
        TokenKind::Less => "<",
        TokenKind::Greater => ">",
        TokenKind::LessEqual => "<=",
        TokenKind::GreaterEqual => ">=",
        TokenKind::EqualEqual => "==",
        TokenKind::BangEqual => "!=",
        TokenKind::Caret => "^",
        TokenKind::Pipe => "|",
        TokenKind::AmpAmp => "&&",
        TokenKind::PipePipe => "||",
        TokenKind::Question => "?",
        TokenKind::Colon => ":",
        TokenKind::Semicolon => ";",
        TokenKind::Ellipsis => "...",
        TokenKind::Equal => "=",
        TokenKind::StarEqual => "*=",
        TokenKind::SlashEqual => "/=",
        TokenKind::PercentEqual => "%=",
        TokenKind::PlusEqual => "+=",
        TokenKind::MinusEqual => "-=",
        TokenKind::LessLessEqual => "<<=",
        TokenKind::GreaterGreaterEqual => ">>=",
        TokenKind::AmpEqual => "&=",
        TokenKind::CaretEqual => "^=",
        TokenKind::PipeEqual => "|=",
        TokenKind::Comma => ",",
        TokenKind::Hash => "#",
        TokenKind::HashHash => "##",
        TokenKind::Identifier(_) => "<identifier>",
        TokenKind::IntegerLiteral { .. } => "<integer>",
        TokenKind::FloatLiteral { .. } => "<float>",
        TokenKind::CharLiteral { .. } => "<char>",
        TokenKind::StringLiteral { .. } => "<string>",
        TokenKind::Eof => "<eof>",
        TokenKind::Unknown(_) => "<unknown>",
    }
}
