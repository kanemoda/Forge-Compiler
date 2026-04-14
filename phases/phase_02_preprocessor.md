# Phase 2 — Preprocessor

**Depends on:** Phase 1 (Lexer)
**Unlocks:** Phase 3 (Parser)
**Estimated duration:** 7–14 days

---

## Goal

Build a complete C17 preprocessor. This is one of the hardest phases because the C preprocessor is essentially a separate language with subtle, specification-defined behavior around macro expansion, token pasting, and conditional compilation. Getting this right is essential — virtually every real-world C file relies heavily on the preprocessor.

---

## Deliverables

1. **`forge_preprocess` crate** — takes a token stream from the lexer, processes all directives, and outputs a clean token stream for the parser
2. **`#include` handling** — both `<...>` and `"..."` forms, with configurable search paths
3. **`#define` / `#undef`** — object-like and function-like macros, including variadic (`__VA_ARGS__`)
4. **Macro expansion** — correct recursive expansion with blue-paint algorithm to prevent infinite recursion
5. **Stringification (`#`) and token pasting (`##`)**
6. **Conditional compilation** — `#if`, `#ifdef`, `#ifndef`, `#elif`, `#else`, `#endif` with constant expression evaluation
7. **Predefined macros** — `__FILE__`, `__LINE__`, `__DATE__`, `__TIME__`, `__STDC__`, `__STDC_VERSION__`, `_Pragma`
8. **`#error`, `#warning`, `#line`, `#pragma`**
9. **Comprehensive tests** including tricky macro expansion edge cases

---

## Technical Design

### Preprocessor Architecture

```
Token stream (from lexer)
    │
    ▼
┌──────────────────────┐
│   Directive Handler   │
│   - #include         │
│   - #define / #undef │
│   - #if / #ifdef ... │
│   - #error, #line... │
└──────────┬───────────┘
           │
           ▼
┌──────────────────────┐
│   Macro Expander     │
│   - Blue-paint algo  │
│   - # stringify      │
│   - ## paste         │
│   - __VA_ARGS__      │
└──────────┬───────────┘
           │
           ▼
Clean token stream (no directives, all macros expanded)
```

### Macro Table

```rust
pub enum MacroDef {
    ObjectLike {
        name: String,
        replacement: Vec<Token>,
    },
    FunctionLike {
        name: String,
        params: Vec<String>,
        is_variadic: bool,  // last param is ...
        replacement: Vec<Token>,
    },
}

pub struct PreprocessorState {
    macros: HashMap<String, MacroDef>,
    include_paths: Vec<PathBuf>,       // search paths for #include
    include_stack: Vec<SourceFile>,     // for detecting circular includes
    if_stack: Vec<IfState>,            // for nested #if tracking
}
```

### Blue-Paint Algorithm (Macro Expansion)

The C standard defines macro expansion in terms of "painting" — when a macro is being expanded, its name is "painted blue" so it won't be expanded again if encountered during its own expansion. This prevents infinite recursion.

Key rules:
1. When expanding macro M, mark M as "blue" (unavailable for expansion)
2. Substitute arguments into the replacement list
3. `#` before a parameter → stringify the argument tokens
4. `##` between tokens → paste the adjacent tokens together, then re-lex
5. Rescan the replacement list for more macros, but skip any blue-painted names
6. After expansion of M is complete, un-paint M

### Conditional Expression Evaluation

`#if` directives need a constant expression evaluator that handles:
- Integer arithmetic: +, -, *, /, %, <<, >>, &, |, ^, ~, !, &&, ||
- Comparison: <, >, <=, >=, ==, !=
- Ternary: ? :
- `defined(X)` and `defined X` operator
- Macro expansion happens first, then undefined identifiers become 0
- All arithmetic is in `intmax_t` (i64 or i128)

---

## Acceptance Criteria

- [ ] `#include <stdio.h>` works with system headers (need to locate system include paths on the host)
- [ ] `#include "local.h"` works relative to the including file
- [ ] Object-like macros: `#define FOO 42` → `FOO` expands to `42`
- [ ] Function-like macros: `#define MAX(a,b) ((a)>(b)?(a):(b))` works correctly
- [ ] Variadic macros: `#define LOG(fmt, ...) printf(fmt, __VA_ARGS__)` works
- [ ] Stringification: `#define STR(x) #x` → `STR(hello)` produces `"hello"`
- [ ] Token pasting: `#define CONCAT(a,b) a##b` → `CONCAT(foo,bar)` produces `foobar`
- [ ] Recursive expansion is correctly prevented (blue paint)
- [ ] `#if`, `#ifdef`, `#elif`, `#else`, `#endif` nest correctly
- [ ] `#if` expressions evaluate correctly including `defined()`
- [ ] `#error` produces a compiler error with the message text
- [ ] `#line` changes reported line numbers
- [ ] `__FILE__`, `__LINE__`, `__DATE__`, `__TIME__` produce correct values
- [ ] Can preprocess real-world headers (try `<stddef.h>`, `<stdint.h>`, `<limits.h>`)

---

## Testing Strategy

1. **Unit tests** for macro expansion engine in isolation
2. **Lit tests** for each directive type
3. **Real header test** — preprocess a file that `#include <stdio.h>` on Ubuntu and verify no errors
4. **Edge case tests** from known preprocessor test suites (the mcpp test suite is a good reference)

---

## Claude Code Prompts

### Prompt 2.1 — Preprocessor skeleton and #define/#undef

```
Create the forge_preprocess crate in the Forge workspace. This implements the C17 preprocessor.

Core structure:
1. A `Preprocessor` struct that takes a token stream (Vec<Token> from forge_lexer) and produces an expanded token stream.
2. A `MacroDef` enum with ObjectLike { name, replacement_tokens } and FunctionLike { name, params, is_variadic, replacement_tokens } variants.
3. A `PreprocessorState` with a HashMap<String, MacroDef> for the macro table.

Implement:
- Scanning through tokens, identifying preprocessor directives (a Hash token that is at_start_of_line followed by an identifier)
- `#define` for object-like macros: `#define NAME replacement-tokens` — collect tokens until end of line
- `#define` for function-like macros: `#define NAME(params) replacement-tokens` — NOTE: the '(' must immediately follow NAME with no space (this distinguishes function-like from object-like)
- Handle variadic macros with `...` as last parameter
- `#undef NAME` — remove from macro table
- Basic macro expansion for object-like macros: when an identifier token matches a defined macro, replace it with the replacement tokens

Do NOT implement function-like macro invocation, stringification, token pasting, or the blue-paint algorithm yet — just object-like expansion.

Write tests:
- Define and expand an object-like macro
- #undef removes a macro
- Unexpanded tokens pass through unchanged
- Distinguish function-like definition (no space before paren) from object-like
```

### Prompt 2.2 — Function-like macro expansion and blue-paint algorithm

```
Extend forge_preprocess with function-like macro expansion and the blue-paint prevention algorithm.

Function-like macro invocation:
1. When an identifier matches a function-like macro AND is followed by '(', collect arguments
2. Arguments are comma-separated token sequences, respecting nested parentheses
3. Substitute each parameter occurrence in the replacement list with the corresponding argument tokens
4. If the macro is variadic, __VA_ARGS__ in the replacement list is replaced with the variadic arguments (comma-separated)

Blue-paint algorithm (prevents infinite recursion):
1. Maintain a "hide set" — a set of macro names currently being expanded
2. When expanding macro M, add M to the hide set
3. During rescanning of replacement tokens, if an identifier matches a macro but that macro name is in the hide set, leave it unexpanded
4. After M's expansion is complete, remove M from the hide set
5. Handle nested macro calls: if A expands to B() and B expands to A, the inner A must not re-expand

Implement stringification (#):
- In a function-like macro replacement list, if # precedes a parameter name, replace # and the parameter with a string literal token containing the argument's tokens as text (with whitespace normalized)

Implement token pasting (##):
- In a replacement list, if ## appears between two tokens, concatenate the spelling of the left and right tokens into a single new token, then re-lex that combined text
- If ## is at the start or end of replacement list, it's an error

Write thorough tests:
- Simple function-like macro: MAX(a,b)
- Nested macro calls: FOO expands to BAR(1), BAR(x) expands to x+1
- Recursive prevention: A defined as A — should not infinite loop
- Mutual recursion: A defined as B, B defined as A
- Stringification with various argument types
- Token pasting creating identifiers, numbers, and punctuators
- Variadic macros with __VA_ARGS__
- Edge case: empty arguments, single argument to multi-param macro
```

### Prompt 2.3 — Conditional compilation

```
Extend forge_preprocess with conditional compilation directives.

Implement:
1. #if <constant-expression>
2. #ifdef <identifier>
3. #ifndef <identifier>
4. #elif <constant-expression>
5. #else
6. #endif

Conditional expression evaluator:
- Before evaluating, expand all macros in the expression
- Replace any remaining identifiers with 0 (per C standard)
- Handle the `defined` operator: `defined(X)` and `defined X` — returns 1 if X is defined, 0 otherwise. Important: `defined` check happens BEFORE macro expansion of the expression
- Parse and evaluate C integer constant expressions supporting:
  - Integer literals (decimal, octal, hex)
  - Unary operators: +, -, ~, !
  - Binary operators: *, /, %, +, -, <<, >>, <, >, <=, >=, ==, !=, &, ^, |, &&, ||
  - Ternary: ? :
  - Parenthesized subexpressions
  - All arithmetic uses i64

If-stack tracking:
- Maintain a stack of IfState { condition_met: bool, else_seen: bool, any_branch_taken: bool }
- When a condition is false, skip all tokens until #elif, #else, or #endif
- Nesting must work correctly
- Error on #else after #else, #elif after #else, unmatched #endif, unterminated #if at end of file

Write tests:
- Simple #ifdef/#ifndef
- #if with arithmetic expressions
- #if with defined() operator
- Nested #if blocks
- #elif chains (first true branch wins)
- #if 0 skipping code
- Error cases: unmatched directives
```

### Prompt 2.4 — #include and predefined macros

```
Extend forge_preprocess with #include handling and predefined macros.

#include implementation:
1. #include <filename> — search system include paths
2. #include "filename" — search relative to current file first, then system paths
3. The preprocessor needs a list of include search paths (passed via configuration)
4. For system headers on Ubuntu, the default paths should include /usr/include and /usr/lib/gcc/x86_64-linux-gnu/*/include (detect the GCC version)
5. Read the included file, lex it, and recursively preprocess the resulting tokens
6. Maintain an include stack to detect circular includes (error if the same file is included while already being processed, unless it has an include guard)
7. #include with a macro argument: expand the macro first, then process the result as < > or " " include

Predefined macros:
- __FILE__ — string literal with current filename
- __LINE__ — integer literal with current line number
- __DATE__ — string literal "Mmm dd yyyy"
- __TIME__ — string literal "hh:mm:ss"
- __STDC__ — 1
- __STDC_VERSION__ — 201710L (for C17)
- __STDC_HOSTED__ — 1

Also implement:
- #error <message> — emit a compile error with the message tokens as text
- #warning <message> — emit a warning (extension, but widely used)
- #line <number> ["filename"] — override the reported line/file
- #pragma — for now, ignore unknown pragmas (emit a note). Handle `#pragma once` by tracking which files have been included.
- _Pragma("string") — convert to equivalent #pragma

Write tests:
- #include "local_file.h" with a test fixture file
- Predefined macros produce correct values
- #error halts with the right message
- #line changes subsequent __LINE__ values
- #pragma once prevents double inclusion
- Circular include detection
```

### Prompt 2.5 — Integration and real-world header testing

```
Integrate the preprocessor into the Forge driver pipeline and test with real system headers.

1. Update forge_driver::compile() to:
   - Lex the source file
   - Preprocess the token stream
   - For `forge check`, print the preprocessed token stream
   - Add a `forge preprocess <file.c>` subcommand that outputs the preprocessed source (like `gcc -E`)

2. Add include path configuration:
   - CLI flags: -I <path> for additional include paths
   - Auto-detect system include paths on Ubuntu by running `gcc -E -v -x c /dev/null 2>&1` and parsing the output

3. Create integration tests that preprocess real files:
   - A test file that #include <stddef.h> — verify it produces tokens (size_t should be defined)
   - A test file that #include <stdint.h> — verify int32_t etc. are defined
   - A test file that #include <limits.h> — verify INT_MAX etc.
   - A test file using conditional compilation with platform macros

4. Add platform predefined macros:
   - __x86_64__ or __aarch64__ depending on target
   - __linux__, __unix__, __gnu_linux__
   - __LP64__
   - __BYTE_ORDER__ (little endian)

5. Run `cargo clippy` and fix all warnings. Ensure all tests pass.

This is a critical milestone: if the preprocessor can handle system headers, the frontend is viable.
```

---

## Notes

- The preprocessor is where most C compilers have the most bugs. Take extra care with macro expansion order — the C standard (§6.10.3) is the source of truth.
- Performance matters here: some C files (especially with system headers) can expand to hundreds of thousands of tokens. The preprocessor should not re-allocate excessively.
- We do NOT handle trigraphs. They were removed in C23 and almost no modern code uses them.
- Line continuations (backslash-newline) should ideally be handled at the lexer level by splicing lines before tokenization. If Phase 1 didn't handle this, it should be added here.
