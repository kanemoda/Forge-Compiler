//! Macro-expansion helpers: spellings, stringification, and token pasting.
//!
//! Function-like macro expansion (C17 §6.10.3) has three pieces that are
//! convenient to write as free functions rather than methods on
//! [`Preprocessor`](crate::Preprocessor):
//!
//! * [`spelling_of`] — reconstruct the canonical source text for a single
//!   [`TokenKind`].  Used by stringification and token pasting alike.
//! * [`stringify`] — build the string-literal value produced by the `#`
//!   operator over the raw (unexpanded) token list of one argument,
//!   escaping every `"` and `\` per C17 §6.10.3.2.
//! * [`paste_spelling`] — concatenate two tokens' spellings for the `##`
//!   operator (the re-lexing step lives in [`crate::preprocessor`] so it
//!   can emit diagnostics on the preprocessor's diagnostics channel).
//!
//! Keeping these pieces pure — no `&mut self`, no diagnostics — makes
//! them easy to unit-test in isolation.

use forge_lexer::{CharPrefix, FloatSuffix, IntSuffix, StringPrefix, Token, TokenKind};

use crate::pp_token::PPToken;

/// Reconstruct the canonical source spelling of a single token kind.
///
/// The returned string is what the lexer would scan back into the same
/// token (modulo suffix case normalisation for numeric literals and
/// deliberate re-escaping for character / string literals).  This is
/// what the preprocessor feeds back through the lexer when it pastes
/// two tokens together, and what it concatenates when it stringifies a
/// macro argument.
///
/// For character and string literals the returned spelling **is the
/// source form** — escape sequences are re-emitted (e.g. a newline
/// in a `StringLiteral` value comes back out as `\n`).  This matches
/// what GCC and Clang do for the `#` operator.
pub fn spelling_of(kind: &TokenKind) -> String {
    match kind {
        // --- Keywords ---
        TokenKind::Auto => "auto".into(),
        TokenKind::Break => "break".into(),
        TokenKind::Case => "case".into(),
        TokenKind::Char => "char".into(),
        TokenKind::Const => "const".into(),
        TokenKind::Continue => "continue".into(),
        TokenKind::Default => "default".into(),
        TokenKind::Do => "do".into(),
        TokenKind::Double => "double".into(),
        TokenKind::Else => "else".into(),
        TokenKind::Enum => "enum".into(),
        TokenKind::Extern => "extern".into(),
        TokenKind::Float => "float".into(),
        TokenKind::For => "for".into(),
        TokenKind::Goto => "goto".into(),
        TokenKind::If => "if".into(),
        TokenKind::Inline => "inline".into(),
        TokenKind::Int => "int".into(),
        TokenKind::Long => "long".into(),
        TokenKind::Register => "register".into(),
        TokenKind::Restrict => "restrict".into(),
        TokenKind::Return => "return".into(),
        TokenKind::Short => "short".into(),
        TokenKind::Signed => "signed".into(),
        TokenKind::Sizeof => "sizeof".into(),
        TokenKind::Static => "static".into(),
        TokenKind::Struct => "struct".into(),
        TokenKind::Switch => "switch".into(),
        TokenKind::Typedef => "typedef".into(),
        TokenKind::Union => "union".into(),
        TokenKind::Unsigned => "unsigned".into(),
        TokenKind::Void => "void".into(),
        TokenKind::Volatile => "volatile".into(),
        TokenKind::While => "while".into(),
        TokenKind::Alignas => "_Alignas".into(),
        TokenKind::Alignof => "_Alignof".into(),
        TokenKind::Atomic => "_Atomic".into(),
        TokenKind::Bool => "_Bool".into(),
        TokenKind::Complex => "_Complex".into(),
        TokenKind::Generic => "_Generic".into(),
        TokenKind::Imaginary => "_Imaginary".into(),
        TokenKind::Noreturn => "_Noreturn".into(),
        TokenKind::StaticAssert => "_Static_assert".into(),
        TokenKind::ThreadLocal => "_Thread_local".into(),

        // --- Punctuators ---
        TokenKind::LeftParen => "(".into(),
        TokenKind::RightParen => ")".into(),
        TokenKind::LeftBrace => "{".into(),
        TokenKind::RightBrace => "}".into(),
        TokenKind::LeftBracket => "[".into(),
        TokenKind::RightBracket => "]".into(),
        TokenKind::Dot => ".".into(),
        TokenKind::Arrow => "->".into(),
        TokenKind::PlusPlus => "++".into(),
        TokenKind::MinusMinus => "--".into(),
        TokenKind::Ampersand => "&".into(),
        TokenKind::Star => "*".into(),
        TokenKind::Plus => "+".into(),
        TokenKind::Minus => "-".into(),
        TokenKind::Tilde => "~".into(),
        TokenKind::Bang => "!".into(),
        TokenKind::Slash => "/".into(),
        TokenKind::Percent => "%".into(),
        TokenKind::LessLess => "<<".into(),
        TokenKind::GreaterGreater => ">>".into(),
        TokenKind::Less => "<".into(),
        TokenKind::Greater => ">".into(),
        TokenKind::LessEqual => "<=".into(),
        TokenKind::GreaterEqual => ">=".into(),
        TokenKind::EqualEqual => "==".into(),
        TokenKind::BangEqual => "!=".into(),
        TokenKind::Caret => "^".into(),
        TokenKind::Pipe => "|".into(),
        TokenKind::AmpAmp => "&&".into(),
        TokenKind::PipePipe => "||".into(),
        TokenKind::Question => "?".into(),
        TokenKind::Colon => ":".into(),
        TokenKind::Semicolon => ";".into(),
        TokenKind::Ellipsis => "...".into(),
        TokenKind::Equal => "=".into(),
        TokenKind::StarEqual => "*=".into(),
        TokenKind::SlashEqual => "/=".into(),
        TokenKind::PercentEqual => "%=".into(),
        TokenKind::PlusEqual => "+=".into(),
        TokenKind::MinusEqual => "-=".into(),
        TokenKind::LessLessEqual => "<<=".into(),
        TokenKind::GreaterGreaterEqual => ">>=".into(),
        TokenKind::AmpEqual => "&=".into(),
        TokenKind::CaretEqual => "^=".into(),
        TokenKind::PipeEqual => "|=".into(),
        TokenKind::Comma => ",".into(),
        TokenKind::Hash => "#".into(),
        TokenKind::HashHash => "##".into(),

        // --- Identifiers and literals ---
        TokenKind::Identifier(s) => s.clone(),
        TokenKind::IntegerLiteral { value, suffix } => {
            format!("{value}{}", int_suffix_spelling(*suffix))
        }
        TokenKind::FloatLiteral { value, suffix } => {
            // `{:?}` keeps the decimal point / exponent, so `3.0` does not
            // round-trip through `{}` to `"3"`.
            let body = format!("{value:?}");
            format!("{body}{}", float_suffix_spelling(*suffix))
        }
        TokenKind::CharLiteral { value, prefix } => format_char_literal(*value, *prefix),
        TokenKind::StringLiteral { value, prefix } => format_string_literal(value, *prefix),

        // --- Sentinels ---
        TokenKind::Eof => String::new(),
        TokenKind::Unknown(c) => c.to_string(),
    }
}

/// Implement the `#` operator: produce the raw text for an argument,
/// then escape every `"` and `\` per C17 §6.10.3.2.
///
/// The `arg` token list is used **verbatim** (no macro expansion on its
/// tokens), preserving the text the user actually wrote.  Leading and
/// trailing whitespace is trimmed; between tokens, a single space is
/// emitted iff the original token had `has_leading_space` set.
pub fn stringify(arg: &[PPToken]) -> String {
    let mut raw = String::new();
    for (i, tok) in arg.iter().enumerate() {
        if i > 0 && tok.has_leading_space() {
            raw.push(' ');
        }
        raw.push_str(&spelling_of(tok.kind()));
    }
    // Now escape for embedding in a C string literal: `\` → `\\`, `"` → `\"`.
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Concatenate the spellings of two tokens for the `##` operator.
///
/// Either side may be `None` (representing the "placeholder token" C17
/// §6.10.3.3 inserts for an empty macro argument); two `None`s produce
/// the empty string.
pub fn paste_spelling(left: Option<&Token>, right: Option<&Token>) -> String {
    let mut out = String::new();
    if let Some(t) = left {
        out.push_str(&spelling_of(&t.kind));
    }
    if let Some(t) = right {
        out.push_str(&spelling_of(&t.kind));
    }
    out
}

// ---------------------------------------------------------------------------
// Internal formatters
// ---------------------------------------------------------------------------

fn int_suffix_spelling(s: IntSuffix) -> &'static str {
    match s {
        IntSuffix::None => "",
        IntSuffix::U => "u",
        IntSuffix::L => "l",
        IntSuffix::UL => "ul",
        IntSuffix::LL => "ll",
        IntSuffix::ULL => "ull",
    }
}

fn float_suffix_spelling(s: FloatSuffix) -> &'static str {
    match s {
        FloatSuffix::None => "",
        FloatSuffix::F => "f",
        FloatSuffix::L => "l",
    }
}

fn char_prefix_spelling(p: CharPrefix) -> &'static str {
    match p {
        CharPrefix::None => "",
        CharPrefix::L => "L",
        CharPrefix::U16 => "u",
        CharPrefix::U32 => "U",
    }
}

fn string_prefix_spelling(p: StringPrefix) -> &'static str {
    match p {
        StringPrefix::None => "",
        StringPrefix::L => "L",
        StringPrefix::Utf8 => "u8",
        StringPrefix::U16 => "u",
        StringPrefix::U32 => "U",
    }
}

/// Re-emit a single character inside a `'...'` character literal.
///
/// Control characters, the escape character itself, and the delimiter
/// (`'`) are re-written as their C source-form escapes.
fn escape_in_char_literal(cp: u32) -> String {
    match cp {
        0x00 => "\\0".into(),
        0x07 => "\\a".into(),
        0x08 => "\\b".into(),
        0x09 => "\\t".into(),
        0x0A => "\\n".into(),
        0x0B => "\\v".into(),
        0x0C => "\\f".into(),
        0x0D => "\\r".into(),
        0x1B => "\\e".into(),
        0x5C => "\\\\".into(),
        0x27 => "\\'".into(),
        x if (0x20..=0x7E).contains(&x) => {
            // Printable ASCII.  Safe to cast because we know x < 0x80.
            (x as u8 as char).to_string()
        }
        x if x <= 0xFF => format!("\\x{x:02x}"),
        x if x <= 0xFFFF => format!("\\u{x:04x}"),
        x => format!("\\U{x:08x}"),
    }
}

/// Re-emit one byte / character inside a `"..."` string literal.
///
/// The ruleset is the same as for character literals except that the
/// single quote does *not* need escaping and the double quote does.
fn escape_in_string_literal(ch: char) -> String {
    match ch {
        '\0' => "\\0".into(),
        '\x07' => "\\a".into(),
        '\x08' => "\\b".into(),
        '\t' => "\\t".into(),
        '\n' => "\\n".into(),
        '\x0B' => "\\v".into(),
        '\x0C' => "\\f".into(),
        '\r' => "\\r".into(),
        '\x1B' => "\\e".into(),
        '\\' => "\\\\".into(),
        '"' => "\\\"".into(),
        c if (0x20..=0x7E).contains(&(c as u32)) => c.to_string(),
        c if (c as u32) <= 0xFF => format!("\\x{:02x}", c as u32),
        c if (c as u32) <= 0xFFFF => format!("\\u{:04x}", c as u32),
        c => format!("\\U{:08x}", c as u32),
    }
}

fn format_char_literal(value: u32, prefix: CharPrefix) -> String {
    let body = escape_in_char_literal(value);
    format!("{}'{body}'", char_prefix_spelling(prefix))
}

fn format_string_literal(value: &str, prefix: StringPrefix) -> String {
    let mut body = String::with_capacity(value.len() + 2);
    for ch in value.chars() {
        body.push_str(&escape_in_string_literal(ch));
    }
    format!("{}\"{body}\"", string_prefix_spelling(prefix))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spelling_of_string_literal_reescapes_specials() {
        let s = format_string_literal("hello\nworld", StringPrefix::None);
        assert_eq!(s, "\"hello\\nworld\"");
    }

    #[test]
    fn spelling_of_string_literal_preserves_prefix() {
        let s = format_string_literal("x", StringPrefix::Utf8);
        assert_eq!(s, "u8\"x\"");
    }

    #[test]
    fn spelling_of_char_literal_escapes_backslash_and_quote() {
        assert_eq!(
            format_char_literal(b'\\' as u32, CharPrefix::None),
            "'\\\\'"
        );
        assert_eq!(format_char_literal(b'\'' as u32, CharPrefix::None), "'\\''");
        assert_eq!(format_char_literal(b'A' as u32, CharPrefix::None), "'A'");
    }
}
