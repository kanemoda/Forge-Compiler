//! The hand-written C17 lexer.
//!
//! The [`Lexer`] owns a borrow of the source text and advances a byte cursor
//! on each call to [`Lexer::next_token`].  It exposes both a batch API
//! ([`Lexer::tokenize`]) and a streaming API ([`Iterator`] impl).
//!
//! # Flag tracking
//!
//! The lexer carries two running flags that are attached to the **next**
//! token emitted:
//!
//! * `at_start_of_line` — set when the lexer is at the start of input or
//!   has just skipped a newline (including newlines inside block comments).
//! * `has_leading_space` — set when any whitespace or comment was skipped
//!   before this token.
//!
//! Both flags are consumed (reset) immediately after the token is built.

use forge_diagnostics::Diagnostic;

use crate::token::{CharPrefix, StringPrefix, Token, TokenKind};
use crate::Span;

/// The C17 lexer.
///
/// Construct with [`Lexer::new`] and drive with either [`Lexer::tokenize`]
/// (returns a `Vec<Token>` terminated by [`TokenKind::Eof`]) or the
/// [`Iterator`] impl (same stream, one token at a time).
///
/// Any non-fatal errors encountered while lexing — invalid octal digits,
/// overflowed integer literals, malformed hex floats, and so on — are
/// recorded on the lexer and can be retrieved after tokenization via
/// [`Lexer::take_diagnostics`].
pub struct Lexer<'a> {
    pub(crate) source: &'a str,
    pub(crate) bytes: &'a [u8],
    pub(crate) pos: usize,
    /// Flag for the next token: first non-whitespace on its line.
    at_start_of_line: bool,
    /// Flag for the next token: was preceded by whitespace/comment.
    has_leading_space: bool,
    /// Iterator guard: once EOF is emitted, `next` returns `None`.
    emitted_eof: bool,
    /// Diagnostics accumulated during lexing.
    ///
    /// Populated by the numeric (and, in later prompts, string/char)
    /// sub-lexers.  Retrieve with [`Lexer::take_diagnostics`].
    pub(crate) diagnostics: Vec<Diagnostic>,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer over `source`.
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            // The first token is by definition at the start of its line.
            at_start_of_line: true,
            has_leading_space: false,
            emitted_eof: false,
            diagnostics: Vec::new(),
        }
    }

    /// Drain and return every [`Diagnostic`] produced so far.
    ///
    /// After this call the internal buffer is empty, so a second call
    /// returns an empty vector unless more tokens are lexed in between.
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    /// A read-only view of diagnostics currently buffered on the lexer.
    ///
    /// Useful when callers want to inspect diagnostics without resetting
    /// the buffer — prefer [`Lexer::take_diagnostics`] when ownership of
    /// the diagnostics is wanted.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Record a [`Diagnostic`] on the lexer.
    ///
    /// Used by the numeric sub-lexer (and, later, the string/char
    /// sub-lexer) to report errors without aborting tokenization.
    pub(crate) fn emit_diagnostic(&mut self, diag: Diagnostic) {
        self.diagnostics.push(diag);
    }

    /// Tokenize the entire source.
    ///
    /// The returned vector always ends with a [`TokenKind::Eof`] token whose
    /// span points at the final byte of input.
    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token();
            let is_eof = matches!(tok.kind, TokenKind::Eof);
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        tokens
    }

    // -----------------------------------------------------------------
    // Core scanner
    // -----------------------------------------------------------------

    /// Scan the next token, consuming any preceding whitespace/comments.
    fn next_token(&mut self) -> Token {
        self.skip_whitespace_and_comments();

        let start = self.pos as u32;
        let at_start_of_line = self.at_start_of_line;
        let has_leading_space = self.has_leading_space;

        let kind = self.lex_kind();

        let end = self.pos as u32;
        let span = Span { start, end };

        // Reset running flags for the *next* token.  We do this even for EOF
        // so repeated `next_token` calls (or a cooperating Iterator impl)
        // produce deterministic flag state.
        self.at_start_of_line = false;
        self.has_leading_space = false;

        Token {
            kind,
            span,
            at_start_of_line,
            has_leading_space,
        }
    }

    /// Dispatch on the current byte to pick the appropriate sub-lexer.
    fn lex_kind(&mut self) -> TokenKind {
        let Some(c) = self.peek() else {
            return TokenKind::Eof;
        };

        match c {
            // Wide / UTF-16 / UTF-32 / UTF-8 character and string prefixes.
            // These must be checked before the generic identifier path so
            // `L'x'`, `u"..."`, etc. are not mis-lexed as identifier + quote.
            b'L' if self.peek_at(1) == Some(b'\'') => {
                self.pos += 1;
                self.lex_char_literal(CharPrefix::L)
            }
            b'L' if self.peek_at(1) == Some(b'"') => {
                self.pos += 1;
                self.lex_string_literal(StringPrefix::L)
            }
            b'u' if self.peek_at(1) == Some(b'\'') => {
                self.pos += 1;
                self.lex_char_literal(CharPrefix::U16)
            }
            b'u' if self.peek_at(1) == Some(b'"') => {
                self.pos += 1;
                self.lex_string_literal(StringPrefix::U16)
            }
            b'u' if self.peek_at(1) == Some(b'8') && self.peek_at(2) == Some(b'"') => {
                self.pos += 2;
                self.lex_string_literal(StringPrefix::Utf8)
            }
            b'U' if self.peek_at(1) == Some(b'\'') => {
                self.pos += 1;
                self.lex_char_literal(CharPrefix::U32)
            }
            b'U' if self.peek_at(1) == Some(b'"') => {
                self.pos += 1;
                self.lex_string_literal(StringPrefix::U32)
            }

            // Unprefixed char and string literals.
            b'\'' => self.lex_char_literal(CharPrefix::None),
            b'"' => self.lex_string_literal(StringPrefix::None),

            // Numeric literal starting with a digit.
            b'0'..=b'9' => self.lex_numeric_literal(),

            // `.` immediately followed by a digit is a fractional-only
            // decimal float (e.g., `.5`, `.25e3`).  Bare `.` (and `...`)
            // stay in the punctuator path below.
            b'.' if matches!(self.peek_at(1), Some(b'0'..=b'9')) => self.lex_numeric_literal(),

            // Identifier or keyword (ASCII letters and underscore).
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.lex_identifier_or_keyword(),

            // Punctuators and anything else.
            _ => self.lex_punctuator_or_unknown(c),
        }
    }

    // -----------------------------------------------------------------
    // Whitespace and comments
    // -----------------------------------------------------------------

    /// Consume whitespace and comments until the next real token or EOF,
    /// updating `at_start_of_line` / `has_leading_space` as side effects.
    fn skip_whitespace_and_comments(&mut self) {
        loop {
            let Some(c) = self.peek() else {
                return;
            };

            match c {
                // Horizontal whitespace.
                b' ' | b'\t' | 0x0B | 0x0C => {
                    self.has_leading_space = true;
                    self.pos += 1;
                }

                // Unix newline.
                b'\n' => {
                    self.at_start_of_line = true;
                    self.has_leading_space = true;
                    self.pos += 1;
                }

                // Classic Mac or Windows newline (CR or CRLF).
                b'\r' => {
                    self.at_start_of_line = true;
                    self.has_leading_space = true;
                    self.pos += 1;
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                }

                // Single-line comment: // ... \n
                b'/' if self.peek_at(1) == Some(b'/') => {
                    self.pos += 2;
                    while let Some(ch) = self.peek() {
                        if ch == b'\n' || ch == b'\r' {
                            break;
                        }
                        self.pos += 1;
                    }
                    self.has_leading_space = true;
                }

                // Line continuation: `\` immediately followed by a newline.
                //
                // Translation phase 2 (C17 §5.1.1.2/1): each
                // backslash-newline pair is deleted, splicing the
                // following physical line onto the current logical line.
                // The splice does **not** mark the next token as being at
                // the start of a line — the logical line has not ended.
                // Any real whitespace around the splice still sets
                // `has_leading_space` on its own, so we do not touch
                // either flag here.
                b'\\' if matches!(self.peek_at(1), Some(b'\n' | b'\r')) => {
                    self.pos += 1; // consume `\`
                    match self.peek() {
                        Some(b'\n') => self.pos += 1,
                        Some(b'\r') => {
                            self.pos += 1;
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        _ => {}
                    }
                }

                // Block comment: /* ... */
                b'/' if self.peek_at(1) == Some(b'*') => {
                    self.pos += 2;
                    loop {
                        match self.peek() {
                            Some(b'*') if self.peek_at(1) == Some(b'/') => {
                                self.pos += 2;
                                break;
                            }
                            Some(b'\n') => {
                                self.at_start_of_line = true;
                                self.pos += 1;
                            }
                            Some(b'\r') => {
                                self.at_start_of_line = true;
                                self.pos += 1;
                                if self.peek() == Some(b'\n') {
                                    self.pos += 1;
                                }
                            }
                            Some(_) => self.pos += 1,
                            // Unterminated block comment.  A later phase will
                            // emit a diagnostic; for now just stop scanning.
                            None => break,
                        }
                    }
                    self.has_leading_space = true;
                }

                _ => return,
            }
        }
    }

    // -----------------------------------------------------------------
    // Identifiers and keywords
    // -----------------------------------------------------------------

    /// Scan an identifier and promote it to a keyword if it matches.
    fn lex_identifier_or_keyword(&mut self) -> TokenKind {
        let start = self.pos;
        // First byte already validated by caller to be [A-Za-z_].
        self.pos += 1;
        while let Some(c) = self.peek() {
            if matches!(c, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = &self.source[start..self.pos];
        lookup_keyword(text).unwrap_or_else(|| TokenKind::Identifier(text.to_string()))
    }

    // -----------------------------------------------------------------
    // Punctuators
    // -----------------------------------------------------------------

    /// Scan a punctuator (longest match) or an unknown character.
    fn lex_punctuator_or_unknown(&mut self, first: u8) -> TokenKind {
        match first {
            // Single-byte unambiguous punctuators.
            b'(' => self.advance1(TokenKind::LeftParen),
            b')' => self.advance1(TokenKind::RightParen),
            b'{' => self.advance1(TokenKind::LeftBrace),
            b'}' => self.advance1(TokenKind::RightBrace),
            b'[' => self.advance1(TokenKind::LeftBracket),
            b']' => self.advance1(TokenKind::RightBracket),
            b'~' => self.advance1(TokenKind::Tilde),
            b';' => self.advance1(TokenKind::Semicolon),
            b',' => self.advance1(TokenKind::Comma),
            b'?' => self.advance1(TokenKind::Question),
            b':' => self.advance1(TokenKind::Colon),

            // `.` or `...`
            b'.' => {
                if self.peek_at(1) == Some(b'.') && self.peek_at(2) == Some(b'.') {
                    self.advance_n(3, TokenKind::Ellipsis)
                } else {
                    self.advance1(TokenKind::Dot)
                }
            }

            // `-`, `->`, `--`, `-=`
            b'-' => match self.peek_at(1) {
                Some(b'>') => self.advance_n(2, TokenKind::Arrow),
                Some(b'-') => self.advance_n(2, TokenKind::MinusMinus),
                Some(b'=') => self.advance_n(2, TokenKind::MinusEqual),
                _ => self.advance1(TokenKind::Minus),
            },

            // `+`, `++`, `+=`
            b'+' => match self.peek_at(1) {
                Some(b'+') => self.advance_n(2, TokenKind::PlusPlus),
                Some(b'=') => self.advance_n(2, TokenKind::PlusEqual),
                _ => self.advance1(TokenKind::Plus),
            },

            // `&`, `&&`, `&=`
            b'&' => match self.peek_at(1) {
                Some(b'&') => self.advance_n(2, TokenKind::AmpAmp),
                Some(b'=') => self.advance_n(2, TokenKind::AmpEqual),
                _ => self.advance1(TokenKind::Ampersand),
            },

            // `|`, `||`, `|=`
            b'|' => match self.peek_at(1) {
                Some(b'|') => self.advance_n(2, TokenKind::PipePipe),
                Some(b'=') => self.advance_n(2, TokenKind::PipeEqual),
                _ => self.advance1(TokenKind::Pipe),
            },

            // `*`, `*=`
            b'*' => match self.peek_at(1) {
                Some(b'=') => self.advance_n(2, TokenKind::StarEqual),
                _ => self.advance1(TokenKind::Star),
            },

            // `/`, `/=`  (block / line comments are consumed earlier.)
            b'/' => match self.peek_at(1) {
                Some(b'=') => self.advance_n(2, TokenKind::SlashEqual),
                _ => self.advance1(TokenKind::Slash),
            },

            // `%`, `%=`
            b'%' => match self.peek_at(1) {
                Some(b'=') => self.advance_n(2, TokenKind::PercentEqual),
                _ => self.advance1(TokenKind::Percent),
            },

            // `^`, `^=`
            b'^' => match self.peek_at(1) {
                Some(b'=') => self.advance_n(2, TokenKind::CaretEqual),
                _ => self.advance1(TokenKind::Caret),
            },

            // `!`, `!=`
            b'!' => match self.peek_at(1) {
                Some(b'=') => self.advance_n(2, TokenKind::BangEqual),
                _ => self.advance1(TokenKind::Bang),
            },

            // `=`, `==`
            b'=' => match self.peek_at(1) {
                Some(b'=') => self.advance_n(2, TokenKind::EqualEqual),
                _ => self.advance1(TokenKind::Equal),
            },

            // `<`, `<<`, `<=`, `<<=`
            b'<' => match self.peek_at(1) {
                Some(b'<') => {
                    if self.peek_at(2) == Some(b'=') {
                        self.advance_n(3, TokenKind::LessLessEqual)
                    } else {
                        self.advance_n(2, TokenKind::LessLess)
                    }
                }
                Some(b'=') => self.advance_n(2, TokenKind::LessEqual),
                _ => self.advance1(TokenKind::Less),
            },

            // `>`, `>>`, `>=`, `>>=`
            b'>' => match self.peek_at(1) {
                Some(b'>') => {
                    if self.peek_at(2) == Some(b'=') {
                        self.advance_n(3, TokenKind::GreaterGreaterEqual)
                    } else {
                        self.advance_n(2, TokenKind::GreaterGreater)
                    }
                }
                Some(b'=') => self.advance_n(2, TokenKind::GreaterEqual),
                _ => self.advance1(TokenKind::Greater),
            },

            // `#`, `##`
            b'#' => match self.peek_at(1) {
                Some(b'#') => self.advance_n(2, TokenKind::HashHash),
                _ => self.advance1(TokenKind::Hash),
            },

            // Anything else — decode as a full UTF-8 char so we consume
            // a whole code-point and preserve source validity.
            _ => {
                let ch = self.consume_unicode_char();
                TokenKind::Unknown(ch)
            }
        }
    }

    // -----------------------------------------------------------------
    // Cursor primitives
    // -----------------------------------------------------------------

    /// Peek the current byte, or `None` at EOF.
    pub(crate) fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Peek `offset` bytes ahead of the current cursor.
    pub(crate) fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    /// Advance one byte and return `kind` — a convenience for single-byte
    /// punctuators.
    fn advance1(&mut self, kind: TokenKind) -> TokenKind {
        self.pos += 1;
        kind
    }

    /// Advance `n` bytes and return `kind`.
    fn advance_n(&mut self, n: usize, kind: TokenKind) -> TokenKind {
        self.pos += n;
        kind
    }

    /// Decode one full UTF-8 code-point at the cursor and advance past it.
    ///
    /// Called only for bytes that don't match any punctuator start so we
    /// don't accidentally split a non-ASCII character mid-sequence.
    ///
    /// If the cursor happens to be at EOF (no caller does this today but
    /// the method is defensive), returns `'\0'` and leaves the cursor
    /// unchanged rather than panicking.
    pub(crate) fn consume_unicode_char(&mut self) -> char {
        match self.source[self.pos..].chars().next() {
            Some(ch) => {
                self.pos += ch.len_utf8();
                ch
            }
            None => '\0',
        }
    }
}

impl Iterator for Lexer<'_> {
    type Item = Token;

    /// Yield the next token, including a single [`TokenKind::Eof`] at end
    /// of input, then `None` thereafter.
    fn next(&mut self) -> Option<Token> {
        if self.emitted_eof {
            return None;
        }
        let tok = self.next_token();
        if matches!(tok.kind, TokenKind::Eof) {
            self.emitted_eof = true;
        }
        Some(tok)
    }
}

// ---------------------------------------------------------------------------
// Keyword lookup
// ---------------------------------------------------------------------------

/// Map an identifier spelling to its keyword [`TokenKind`], if any.
///
/// Public so the preprocessor (and any other downstream consumer that
/// rebuilds identifier tokens after macro expansion) can re-classify a
/// string as a keyword without re-invoking the full lexer.
///
/// A simple `match` outperforms a `HashMap` for small keyword sets — the
/// compiler builds it into a jump table.
pub fn lookup_keyword(text: &str) -> Option<TokenKind> {
    Some(match text {
        "auto" => TokenKind::Auto,
        "break" => TokenKind::Break,
        "case" => TokenKind::Case,
        "char" => TokenKind::Char,
        "const" => TokenKind::Const,
        "continue" => TokenKind::Continue,
        "default" => TokenKind::Default,
        "do" => TokenKind::Do,
        "double" => TokenKind::Double,
        "else" => TokenKind::Else,
        "enum" => TokenKind::Enum,
        "extern" => TokenKind::Extern,
        "float" => TokenKind::Float,
        "for" => TokenKind::For,
        "goto" => TokenKind::Goto,
        "if" => TokenKind::If,
        "inline" => TokenKind::Inline,
        "int" => TokenKind::Int,
        "long" => TokenKind::Long,
        "register" => TokenKind::Register,
        "restrict" => TokenKind::Restrict,
        "return" => TokenKind::Return,
        "short" => TokenKind::Short,
        "signed" => TokenKind::Signed,
        "sizeof" => TokenKind::Sizeof,
        "static" => TokenKind::Static,
        "struct" => TokenKind::Struct,
        "switch" => TokenKind::Switch,
        "typedef" => TokenKind::Typedef,
        "union" => TokenKind::Union,
        "unsigned" => TokenKind::Unsigned,
        "void" => TokenKind::Void,
        "volatile" => TokenKind::Volatile,
        "while" => TokenKind::While,
        "_Alignas" => TokenKind::Alignas,
        "_Alignof" => TokenKind::Alignof,
        "_Atomic" => TokenKind::Atomic,
        "_Bool" => TokenKind::Bool,
        "_Complex" => TokenKind::Complex,
        "_Generic" => TokenKind::Generic,
        "_Imaginary" => TokenKind::Imaginary,
        "_Noreturn" => TokenKind::Noreturn,
        "_Static_assert" => TokenKind::StaticAssert,
        "_Thread_local" => TokenKind::ThreadLocal,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Fragment helper
// ---------------------------------------------------------------------------

/// Lex a short text fragment into tokens, with no file context.
///
/// Used by the preprocessor to re-lex the concatenated spelling produced
/// by a `##` token-pasting operator: the text is not a whole file, there
/// is no preceding context to carry, and the trailing [`TokenKind::Eof`]
/// sentinel that [`Lexer::tokenize`] tacks on would be a nuisance for
/// callers that only want the substantive tokens.
///
/// The returned vector is exactly the non-[`TokenKind::Eof`] tokens the
/// lexer would have produced for `input`, in order.  Diagnostics
/// accumulated during the scan are discarded; a caller that needs them
/// can build a [`Lexer`] directly.
///
/// # Examples
///
/// ```
/// use forge_lexer::{lex_fragment, TokenKind};
///
/// let toks = lex_fragment("foo");
/// assert_eq!(toks.len(), 1);
/// assert!(matches!(toks[0].kind, TokenKind::Identifier(ref s) if s == "foo"));
///
/// // Empty fragments produce zero tokens.
/// assert!(lex_fragment("").is_empty());
/// ```
pub fn lex_fragment(input: &str) -> Vec<Token> {
    let mut lexer = Lexer::new(input);
    let mut tokens = lexer.tokenize();
    if matches!(tokens.last().map(|t| &t.kind), Some(TokenKind::Eof)) {
        tokens.pop();
    }
    tokens
}
