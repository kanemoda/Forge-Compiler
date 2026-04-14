# Phase 1 — Lexer

**Depends on:** Phase 0 (Scaffolding)
**Unlocks:** Phase 2 (Preprocessor)
**Estimated duration:** 3–5 days

---

## Goal

Build a complete C17 lexer that converts source text into a stream of tokens. The lexer must handle all C17 token types, track source positions precisely (for diagnostics), and be fast enough to tokenize large translation units without noticeable delay.

---

## Deliverables

1. **`forge_lexer` crate** with a public `lex(source: &str) -> Vec<Token>` function (and a streaming iterator variant)
2. **Token type** covering all C17 tokens: keywords, identifiers, integer/float/char/string literals, punctuators, and preprocessor-relevant tokens
3. **Source spans** — every token carries a `Span` (byte offset range) for diagnostics
4. **Diagnostics** — lexer errors (unterminated string, invalid numeric literal, unknown character) produce `Diagnostic` values via `forge_diagnostics`
5. **Comprehensive tests** — unit tests for each token category + lit-style test files

---

## Token Categories

### Keywords (C17 complete set)
```
auto, break, case, char, const, continue, default, do, double, else, enum,
extern, float, for, goto, if, inline, int, long, register, restrict, return,
short, signed, sizeof, static, struct, switch, typedef, union, unsigned, void,
volatile, while, _Alignas, _Alignof, _Atomic, _Bool, _Complex, _Generic,
_Imaginary, _Noreturn, _Static_assert, _Thread_local
```

### Punctuators
```
[ ] ( ) { } . -> ++ -- & * + - ~ ! / % << >> < > <= >= == != ^ | && || ? : ;
... = *= /= %= += -= <<= >>= &= ^= |= , # ##
```

### Literals
- **Integer:** decimal, octal (0...), hex (0x...), with suffixes (u, l, ul, ll, ull, etc.)
- **Float:** decimal float, hex float (0x...p...), with suffixes (f, l)
- **Character:** 'x', '\n', '\x41', '\0', L'x', u'x', U'x'
- **String:** "...", L"...", u8"...", u"...", U"..." with all escape sequences

### Other
- **Identifiers** (including those starting with _ that aren't keywords)
- **Preprocessor directives** — at this stage, the lexer produces `#` as a punctuator token and the directive name as an identifier. The preprocessor (Phase 2) interprets them.
- **Comments** — `//` and `/* */` are skipped (not emitted as tokens) but their spans are tracked for correct line/column tracking
- **Whitespace** — skipped but tracked for line counting. A `preceding_whitespace` or `at_start_of_line` flag on tokens is useful for the preprocessor.

---

## Technical Design

### Data Structures

```rust
/// A span in the source text (byte offsets)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,  // byte offset — u32 supports files up to 4GB
    pub end: u32,
}

/// A single token
#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub at_start_of_line: bool,     // needed by preprocessor
    pub has_leading_space: bool,    // needed by preprocessor for ## pasting
}

/// Token kinds — keep this as a flat enum for match performance
#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    // Literals
    IntegerLiteral { value: u64, suffix: IntSuffix },
    FloatLiteral { value: f64, suffix: FloatSuffix },
    CharLiteral { value: u32, prefix: CharPrefix },
    StringLiteral { value: String, prefix: StringPrefix },

    // Identifier or keyword
    Identifier(String),   // keywords are identifiers checked via lookup
    Keyword(Keyword),

    // Punctuators (one variant per punctuator for fast matching)
    LeftParen,    // (
    RightParen,   // )
    LeftBrace,    // {
    RightBrace,   // }
    LeftBracket,  // [
    RightBracket, // ]
    // ... all punctuators ...
    Hash,         // #
    HashHash,     // ##

    // Special
    Eof,
    Unknown(char),  // for error recovery
}
```

### Lexer Architecture

- **Hand-written** — no lexer generators. A `Lexer` struct holds a reference to the source `&str`, current byte position, and produces tokens on demand.
- **Peek-based** — the lexer peeks at the current and next characters to determine token kind. For multi-character tokens (like `>>=`), it greedily matches the longest valid token.
- **UTF-8 aware** — C source is ASCII for syntax, but string/char literals and comments can contain arbitrary UTF-8. The lexer operates on bytes for speed but validates UTF-8 in identifiers.
- **Zero-copy for identifiers** — use `&str` slices into the source where possible, or intern strings.

### Keyword Lookup

Use a perfect hash or a simple `match` on the identifier string. A `phf` crate-based static map is fast and clean. Alternatively, a manual `match` works fine for ~45 keywords.

---

## Acceptance Criteria

- [ ] All C17 keywords are recognized correctly
- [ ] Integer literals: decimal, octal, hex, with all suffix combinations
- [ ] Float literals: decimal and hex float with suffixes
- [ ] Character literals: all prefixes and escape sequences including \x, \u, \U, \0, \n, etc.
- [ ] String literals: all prefixes and escape sequences, adjacent string concatenation is NOT done here (that's the preprocessor/parser)
- [ ] All punctuators, including trigraphs if we support them (optional — most compilers have dropped them)
- [ ] Comments (both styles) are correctly skipped
- [ ] Line/column tracking is correct (verified by span positions in tests)
- [ ] Error recovery: unterminated string/char/comment produces a diagnostic and the lexer continues
- [ ] Performance: tokenize a 50K-line C file in under 50ms

---

## Testing Strategy

1. **Unit tests** in `forge_lexer/src/tests.rs` — one test per token category, edge cases
2. **Lit tests** in `tests/lit/lexer/` — .c files with `// CHECK:` comments verifying token output
3. **Fuzz-adjacent** — test with known tricky inputs: max-length identifiers, deeply nested comments (not valid C but good error test), numeric literal edge cases

---

## Claude Code Prompts

### Prompt 1.1 — Core lexer structure and basic tokens

```
Create the forge_lexer crate in the Forge compiler workspace. This is a C17 lexer.

Define these core types in forge_lexer/src/lib.rs (or split into modules):

1. `Span` — { start: u32, end: u32 } representing byte offsets. Implement Display to show "start..end".

2. `Token` — { kind: TokenKind, span: Span, at_start_of_line: bool, has_leading_space: bool }

3. `TokenKind` — an enum with variants for:
   - Every C17 keyword (auto, break, case, char, const, continue, default, do, double, else, enum, extern, float, for, goto, if, inline, int, long, register, restrict, return, short, signed, sizeof, static, struct, switch, typedef, union, unsigned, void, volatile, while, _Alignas, _Alignof, _Atomic, _Bool, _Complex, _Generic, _Imaginary, _Noreturn, _Static_assert, _Thread_local)
   - Every C17 punctuator as individual variants (LeftParen, RightParen, LeftBrace, RightBrace, LeftBracket, RightBracket, Dot, Arrow, PlusPlus, MinusMinus, Ampersand, Star, Plus, Minus, Tilde, Bang, Slash, Percent, LessLess, GreaterGreater, Less, Greater, LessEqual, GreaterEqual, EqualEqual, BangEqual, Caret, Pipe, AmpAmp, PipePipe, Question, Colon, Semicolon, Ellipsis, Equal, StarEqual, SlashEqual, PercentEqual, PlusEqual, MinusEqual, LessLessEqual, GreaterGreaterEqual, AmpEqual, CaretEqual, PipeEqual, Comma, Hash, HashHash)
   - Identifier(String)
   - IntegerLiteral { value: u64, suffix: IntSuffix }
   - FloatLiteral { value: f64, suffix: FloatSuffix }
   - CharLiteral { value: u32, prefix: CharPrefix }
   - StringLiteral { value: String, prefix: StringPrefix }
   - Eof
   - Unknown(char)

4. Supporting enums: IntSuffix (None, U, L, UL, LL, ULL), FloatSuffix (None, F, L), CharPrefix (None, L, U16, U32), StringPrefix (None, L, Utf8, U16, U32)

5. A `Lexer` struct that takes &str source and has a method `fn tokenize(&mut self) -> Vec<Token>` and also implements Iterator<Item = Token>.

For now, implement tokenization for:
- Whitespace and newline skipping (tracking at_start_of_line and has_leading_space)
- Single-line (//) and multi-line (/* */) comment skipping
- All punctuators (use greedy longest-match)
- Identifiers and keyword recognition (use a match statement or HashMap for keyword lookup)
- Eof

Do NOT implement numeric or string/char literals yet — we'll add those next. For now, if the lexer encounters a digit or quote, emit Unknown.

Write thorough unit tests: test each punctuator, test keywords vs identifiers, test comment skipping, test at_start_of_line tracking.

Add forge_lexer to the workspace Cargo.toml. Add it as a dependency of forge_driver.
```

### Prompt 1.2 — Numeric literals

```
Extend the forge_lexer to handle all C17 numeric literals.

Integer literals:
- Decimal: [1-9][0-9]* or just 0
- Octal: 0[0-7]*
- Hexadecimal: 0[xX][0-9a-fA-F]+
- Suffixes (case-insensitive): u, l, ul, lu, ll, ull, llu — parse all valid combinations
- Compute the u64 value during lexing; if the number overflows u64, emit a diagnostic warning

Float literals:
- Decimal: digits with a dot and/or exponent ([eE][+-]?digits)
  - At least one digit before or after the dot: "1.", ".5", "1.5", "1e10", "1.5e-3"
- Hexadecimal: 0[xX] hex-digits [.hex-digits] p[+-]?digits (the binary exponent is mandatory for hex floats)
- Suffixes (case-insensitive): f (float), l (long double)
- Store as f64 value

Important edge cases:
- "0" is a decimal zero, not an octal literal
- "08" and "09" are invalid octal — emit a diagnostic
- ".5" is a valid float, but "." alone is the Dot punctuator
- Distinguish between "1.method" (int, dot, identifier) and "1.5" (float) — look ahead for digits after dot

Write comprehensive tests for each numeric format, all suffix combinations, overflow cases, and the tricky edge cases listed above.
```

### Prompt 1.3 — String and character literals

```
Extend the forge_lexer to handle all C17 string and character literals.

Character literals:
- Basic: 'x'
- With prefix: L'x', u'x' (lowercase u), U'x' (uppercase U)
- Escape sequences: \a \b \f \n \r \t \v \\ \' \" \?
- Octal escapes: \0, \012, \377 (1-3 octal digits)
- Hex escapes: \x41 (any number of hex digits)
- Universal character names: \u0041 (exactly 4 hex digits), \U00000041 (exactly 8 hex digits)
- Empty character literal '' is an error
- Multi-character literal like 'ab' is implementation-defined but valid — store the value

String literals:
- Basic: "hello"
- With prefix: L"...", u8"...", u"...", U"..."
- Same escape sequences as character literals
- Unterminated string at end of line is an error (C doesn't allow unescaped newlines in strings)
- Handle escaped newline (backslash-newline) for line continuation WITHIN string literals

Error recovery:
- Unterminated character/string literal: emit a diagnostic, consume to end of line or closing quote, and produce the token with what was consumed
- Invalid escape sequence: emit a diagnostic but continue parsing the literal

Write tests for: every escape sequence, every prefix, unterminated literals, invalid escapes, empty char literal, multi-character literals, hex/octal escapes at boundary values.
```

### Prompt 1.4 — Wire lexer into the driver and add lit tests

```
Now integrate the lexer into the Forge compiler pipeline:

1. In forge_driver, update the compile() function to:
   - Call the lexer on the source input
   - If there are lexer diagnostics, render them using forge_diagnostics
   - For now, the `check` command should print the token stream (one token per line: "KIND span=START..END 'text'")

2. Create lit tests in tests/lit/lexer/:
   - tests/lit/lexer/keywords.c — contains all C17 keywords as identifiers, checks they tokenize as keywords
   - tests/lit/lexer/integers.c — various integer literals with expected token output
   - tests/lit/lexer/floats.c — various float literals
   - tests/lit/lexer/strings.c — string and char literals with escapes
   - tests/lit/lexer/punctuators.c — all punctuators
   - tests/lit/lexer/errors.c — unterminated strings, invalid octal, etc. with expected ERROR diagnostics

3. Verify the lit test runner from Phase 0 picks these up and they pass with `cargo test`.

4. Run `cargo clippy` and fix any warnings.
```

---

## Notes

- The lexer does NOT handle `#include` or macro expansion. It just produces `Hash` tokens and identifier tokens for directive names. The preprocessor (Phase 2) will consume the token stream and handle directives.
- String concatenation (`"hello" " world"`) is NOT done by the lexer. That's the preprocessor or parser's job.
- We intentionally store at_start_of_line and has_leading_space because the C preprocessor grammar is whitespace-sensitive (e.g., `#define` must start at beginning of line after `#`, and `## ` token pasting depends on spacing).
