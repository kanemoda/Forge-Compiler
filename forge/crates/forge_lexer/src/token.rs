//! Token type and its supporting enums.
//!
//! The [`TokenKind`] enum is intentionally flat: every C17 keyword and every
//! punctuator is its own variant.  This is verbose but pattern matching is
//! fast and readable in downstream crates (parser, preprocessor) that care
//! about specific tokens.

use crate::Span;

/// A single lexed token, with source span and preprocessor-relevant flags.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    /// What kind of token this is.
    pub kind: TokenKind,
    /// Byte range in the source this token covers.
    pub span: Span,
    /// `true` if this is the first non-whitespace token on its line.
    ///
    /// Used by the preprocessor to recognise directive lines: a `#` with
    /// `at_start_of_line == true` begins a preprocessing directive.
    pub at_start_of_line: bool,
    /// `true` if whitespace or a comment preceded this token.
    ///
    /// Used by the preprocessor for the `##` token-pasting operator, which
    /// must distinguish `a##b` (pasted) from `a ## b` (still pasted — the
    /// flag is informational for diagnostics and macro-expansion fidelity).
    pub has_leading_space: bool,
}

/// The kind of a token.
///
/// C17 keywords are modelled as individual variants (`Auto`, `Break`, …,
/// `StaticAssert`, `ThreadLocal`) and so are punctuators (`LeftParen`,
/// `PlusEqual`, …) to keep pattern matching flat and fast.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    // -----------------------------------------------------------------
    // C11/C17 keywords
    // -----------------------------------------------------------------
    /// `auto`
    Auto,
    /// `break`
    Break,
    /// `case`
    Case,
    /// `char`
    Char,
    /// `const`
    Const,
    /// `continue`
    Continue,
    /// `default`
    Default,
    /// `do`
    Do,
    /// `double`
    Double,
    /// `else`
    Else,
    /// `enum`
    Enum,
    /// `extern`
    Extern,
    /// `float`
    Float,
    /// `for`
    For,
    /// `goto`
    Goto,
    /// `if`
    If,
    /// `inline`
    Inline,
    /// `int`
    Int,
    /// `long`
    Long,
    /// `register`
    Register,
    /// `restrict`
    Restrict,
    /// `return`
    Return,
    /// `short`
    Short,
    /// `signed`
    Signed,
    /// `sizeof`
    Sizeof,
    /// `static`
    Static,
    /// `struct`
    Struct,
    /// `switch`
    Switch,
    /// `typedef`
    Typedef,
    /// `union`
    Union,
    /// `unsigned`
    Unsigned,
    /// `void`
    Void,
    /// `volatile`
    Volatile,
    /// `while`
    While,
    /// `_Alignas`
    Alignas,
    /// `_Alignof`
    Alignof,
    /// `_Atomic`
    Atomic,
    /// `_Bool`
    Bool,
    /// `_Complex`
    Complex,
    /// `_Generic`
    Generic,
    /// `_Imaginary`
    Imaginary,
    /// `_Noreturn`
    Noreturn,
    /// `_Static_assert`
    StaticAssert,
    /// `_Thread_local`
    ThreadLocal,

    // -----------------------------------------------------------------
    // Punctuators
    // -----------------------------------------------------------------
    /// `(`
    LeftParen,
    /// `)`
    RightParen,
    /// `{`
    LeftBrace,
    /// `}`
    RightBrace,
    /// `[`
    LeftBracket,
    /// `]`
    RightBracket,
    /// `.`
    Dot,
    /// `->`
    Arrow,
    /// `++`
    PlusPlus,
    /// `--`
    MinusMinus,
    /// `&`
    Ampersand,
    /// `*`
    Star,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `~`
    Tilde,
    /// `!`
    Bang,
    /// `/`
    Slash,
    /// `%`
    Percent,
    /// `<<`
    LessLess,
    /// `>>`
    GreaterGreater,
    /// `<`
    Less,
    /// `>`
    Greater,
    /// `<=`
    LessEqual,
    /// `>=`
    GreaterEqual,
    /// `==`
    EqualEqual,
    /// `!=`
    BangEqual,
    /// `^`
    Caret,
    /// `|`
    Pipe,
    /// `&&`
    AmpAmp,
    /// `||`
    PipePipe,
    /// `?`
    Question,
    /// `:`
    Colon,
    /// `;`
    Semicolon,
    /// `...`
    Ellipsis,
    /// `=`
    Equal,
    /// `*=`
    StarEqual,
    /// `/=`
    SlashEqual,
    /// `%=`
    PercentEqual,
    /// `+=`
    PlusEqual,
    /// `-=`
    MinusEqual,
    /// `<<=`
    LessLessEqual,
    /// `>>=`
    GreaterGreaterEqual,
    /// `&=`
    AmpEqual,
    /// `^=`
    CaretEqual,
    /// `|=`
    PipeEqual,
    /// `,`
    Comma,
    /// `#`
    Hash,
    /// `##`
    HashHash,

    // -----------------------------------------------------------------
    // Identifiers and literals
    // -----------------------------------------------------------------
    /// A user identifier (never a keyword).
    Identifier(String),
    /// An integer literal and its suffix.
    ///
    /// *Not implemented in Phase 1.1* — numeric lexing is deferred to a
    /// later prompt.  Reserved here so downstream code can be written
    /// against the final enum shape.
    IntegerLiteral {
        /// Parsed value.
        value: u64,
        /// Optional integer suffix.
        suffix: IntSuffix,
    },
    /// A floating-point literal and its suffix.
    ///
    /// *Not implemented in Phase 1.1.*
    FloatLiteral {
        /// Parsed value.
        value: f64,
        /// Optional float suffix.
        suffix: FloatSuffix,
    },
    /// A character literal and its prefix.
    ///
    /// *Not implemented in Phase 1.1.*
    CharLiteral {
        /// Parsed code-point (may exceed 0xFFFF for U prefix).
        value: u32,
        /// Optional character prefix.
        prefix: CharPrefix,
    },
    /// A string literal and its prefix.
    ///
    /// *Not implemented in Phase 1.1.*
    StringLiteral {
        /// Parsed contents (UTF-8).
        value: String,
        /// Optional string prefix.
        prefix: StringPrefix,
    },

    // -----------------------------------------------------------------
    // Sentinels
    // -----------------------------------------------------------------
    /// End of input.  Always emitted as the final token.
    Eof,
    /// A character the lexer does not yet understand.
    ///
    /// Used for error recovery and for parts of the language that the
    /// current phase has not yet implemented (digits, quotes in Phase 1.1).
    Unknown(char),
}

impl TokenKind {
    /// If `self` has an identifier-like spelling, return it.
    ///
    /// At the preprocessor level C has no reserved words — keywords are
    /// just identifiers that happen to have special meaning to the
    /// parser.  `#define`, `#undef`, `defined`, `#ifdef`, macro expansion
    /// and the "unknown identifier becomes 0" rule in `#if` all need to
    /// treat every keyword token the same way they treat a user-written
    /// [`TokenKind::Identifier`].  This method is the single place that
    /// maps a keyword token back to its source spelling for those uses.
    ///
    /// Returns `None` for punctuators, literals, and sentinels.
    ///
    /// # Examples
    ///
    /// ```
    /// use forge_lexer::TokenKind;
    ///
    /// assert_eq!(TokenKind::Int.identifier_spelling(), Some("int"));
    /// assert_eq!(TokenKind::Noreturn.identifier_spelling(), Some("_Noreturn"));
    /// assert_eq!(
    ///     TokenKind::Identifier("foo".into()).identifier_spelling(),
    ///     Some("foo"),
    /// );
    /// assert_eq!(TokenKind::Plus.identifier_spelling(), None);
    /// ```
    pub fn identifier_spelling(&self) -> Option<&str> {
        Some(match self {
            TokenKind::Identifier(s) => s.as_str(),
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
            _ => return None,
        })
    }

    /// `true` iff this token has an identifier-like spelling (either a
    /// user [`TokenKind::Identifier`] or any C keyword).
    ///
    /// Shorthand for `self.identifier_spelling().is_some()`.
    pub fn is_identifier_like(&self) -> bool {
        self.identifier_spelling().is_some()
    }
}

/// Integer-literal suffix.
///
/// `unsigned long int` and `long unsigned int` both map to [`IntSuffix::UL`];
/// `ULL` covers `llu` / `ull` / `llU` / etc.  The suffix is stored in a
/// canonical form so downstream code does not need to re-parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntSuffix {
    /// No suffix.
    None,
    /// `u` or `U`.
    U,
    /// `l` or `L`.
    L,
    /// `ul`, `UL`, `lu`, `Lu`, … (any case, any order).
    UL,
    /// `ll` or `LL`.
    LL,
    /// `ull`, `llu`, … (any case, any order).
    ULL,
}

/// Floating-point literal suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FloatSuffix {
    /// No suffix (type `double`).
    None,
    /// `f` or `F` (type `float`).
    F,
    /// `l` or `L` (type `long double`).
    L,
}

/// Character-literal prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CharPrefix {
    /// No prefix — type `int` holding one narrow char.
    None,
    /// `L'...'` — `wchar_t`.
    L,
    /// `u'...'` — `char16_t`.
    U16,
    /// `U'...'` — `char32_t`.
    U32,
}

/// String-literal prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StringPrefix {
    /// No prefix — narrow string.
    None,
    /// `L"..."` — wide string.
    L,
    /// `u8"..."` — UTF-8 string (C11).
    Utf8,
    /// `u"..."` — UTF-16 string.
    U16,
    /// `U"..."` — UTF-32 string.
    U32,
}
