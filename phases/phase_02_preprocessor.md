# Phase 2 — Preprocessor (Revised)

**Depends on:** Phase 1 (Lexer) ✅ COMPLETE
**Unlocks:** Phase 3 (Parser)
**Estimated duration:** 10–18 days

---

## Goal

Build a complete C17 preprocessor. This is the hardest phase of the frontend because the C preprocessor is effectively a separate language layered on top of C, with its own grammar, its own evaluation rules, and decades of accumulated edge cases.

The preprocessor consumes the lexer's token stream and produces a new token stream with all directives processed, all macros expanded, and all conditional blocks resolved. The parser (Phase 3) never sees a `#` directive.

---

## Why This Phase Is Hard

1. **Macro expansion order is subtle.** The C standard (§6.10.3) defines a specific order: argument substitution happens BEFORE rescan, but `#` and `##` operators apply to the raw (unexpanded) argument tokens. Getting this wrong produces silent miscompilations.

2. **The blue-paint (hide-set) algorithm is easy to get wrong.** It's not just "don't expand X while expanding X." The hide set is per-token, not global — a token carries the set of macros that were being expanded when it was produced. This matters for mutual recursion.

3. **`#include` requires file I/O and recursive preprocessing.** The preprocessor must lex a new file, preprocess it (which may trigger more includes), and splice the result into the current token stream. Circular include detection and `#pragma once` add complexity.

4. **Conditional compilation (`#if`) needs its own expression evaluator** that operates on preprocessor tokens, not AST nodes. This evaluator must handle `defined()`, macro expansion within the expression, and integer-only arithmetic.

5. **Real-world C code abuses the preprocessor.** System headers use every obscure feature. If `#include <stdio.h>` doesn't work, the compiler is useless.

---

## Deliverables

1. **`forge_preprocess` crate** — complete C17 preprocessor
2. **Macro table** — object-like and function-like macros with correct storage
3. **Macro expansion engine** — blue-paint algorithm with correct argument prescan
4. **Stringification (`#`) and token pasting (`##`)**
5. **Conditional compilation** — `#if`/`#ifdef`/`#ifndef`/`#elif`/`#else`/`#endif` with expression evaluation
6. **`#include`** — both `<...>` and `"..."` forms with search path resolution
7. **Predefined macros** — `__FILE__`, `__LINE__`, `__DATE__`, `__TIME__`, `__STDC__`, `__STDC_VERSION__`
8. **`#error`, `#warning`, `#line`, `#pragma`, `_Pragma`**
9. **Comprehensive tests** — unit tests, lit tests, and real system header tests

---

## Technical Design

### Architecture

```
Input: Vec<Token> from forge_lexer
                │
                ▼
┌──────────────────────────────┐
│      Token Cursor            │
│  (peek, advance, push-back)  │
│  Supports injecting tokens   │
│  from #include and macro     │
│  expansion                   │
└──────────────┬───────────────┘
               │
               ▼
┌──────────────────────────────┐
│      Directive Dispatch      │
│  if token is # at SOL:       │
│    parse directive name      │
│    dispatch to handler       │
│  else:                       │
│    try macro expansion       │
│    emit to output            │
└──────────────┬───────────────┘
               │
               ▼
         Vec<Token> output
         (no # directives,
          all macros expanded)
```

### Key Data Structures

```rust
/// A macro definition
pub enum MacroDef {
    ObjectLike {
        name: String,
        replacement: Vec<Token>,
        is_predefined: bool,  // __LINE__ etc. need special handling
    },
    FunctionLike {
        name: String,
        params: Vec<String>,
        is_variadic: bool,    // has ... as last param
        replacement: Vec<Token>,
    },
}

/// State for conditional compilation
struct IfState {
    /// Have we found a true branch yet?
    any_branch_taken: bool,
    /// Is the current branch active (emitting tokens)?
    current_branch_active: bool,
    /// Have we seen #else? (to detect duplicate #else)
    else_seen: bool,
    /// Location of the #if (for error messages on unmatched)
    if_location: Span,
}

/// Main preprocessor state
pub struct Preprocessor {
    macros: HashMap<String, MacroDef>,
    include_paths: Vec<PathBuf>,
    include_stack: Vec<IncludeFrame>,  // for circular detection
    if_stack: Vec<IfState>,
    output: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
}

/// Tracks position in a file being preprocessed
struct IncludeFrame {
    filename: String,
    tokens: Vec<Token>,
    position: usize,
}
```

### Macro Expansion — The Full Algorithm

This is the core of the preprocessor. The C standard (§6.10.3.1 through §6.10.3.4) defines this precisely. Here is the algorithm in detail:

**For object-like macros (`#define FOO bar`):**
1. When identifier `FOO` is encountered:
2. If `FOO` is in the current token's hide-set, skip (don't expand)
3. Take the replacement token list
4. Add `FOO` to the hide-set of every token in the replacement
5. Rescan the replacement for more macros

**For function-like macros (`#define FOO(a, b) a + b`):**
1. When identifier `FOO` is encountered AND followed by `(`
2. If `FOO` is in the current token's hide-set, skip
3. Collect arguments (comma-separated, respecting nested parens)
4. **For each parameter in the replacement list:**
   - If preceded by `#` → stringify the RAW (unexpanded) argument
   - If adjacent to `##` → use the RAW (unexpanded) argument
   - Otherwise → FULLY EXPAND the argument first, then substitute
5. After substitution, process all `##` operators (paste adjacent tokens)
6. Add `FOO` to the hide-set of every token in the result
7. Rescan for more macros

**CRITICAL: The order matters.** Steps 4 must distinguish between `#`/`##` contexts (use raw args) and normal contexts (use expanded args). This is the #1 source of bugs in preprocessor implementations.

### Hide-Set (Blue Paint) — Per-Token

Each token carries a set of macro names that cannot be expanded from that token's perspective:

```rust
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub at_start_of_line: bool,
    pub has_leading_space: bool,
    pub hide_set: HashSet<String>,  // macros that cannot expand this token
}
```

When macro M produces replacement tokens, M is added to each token's hide-set. When two tokens are pasted with `##`, the resulting token gets the INTERSECTION of both hide-sets.

**NOTE:** If the lexer's Token struct doesn't have a `hide_set` field, it needs to be added — either to Token directly or via a wrapper type used only in the preprocessor.

### Conditional Expression Evaluation

The `#if` expression evaluator processes preprocessor tokens (not AST nodes):

1. First, expand all macros in the expression (EXCEPT inside `defined()` — `defined` suppresses expansion of its argument)
2. Replace any remaining identifiers with `0` (per C standard §6.10.1/4)
3. Parse and evaluate the resulting integer constant expression
4. All arithmetic is signed 64-bit (`i64`), matching `intmax_t` on LP64

Supported operators (by precedence, low to high):
- `||` (logical or)
- `&&` (logical and)
- `|` (bitwise or)
- `^` (bitwise xor)
- `&` (bitwise and)
- `==`, `!=`
- `<`, `>`, `<=`, `>=`
- `<<`, `>>`
- `+`, `-`
- `*`, `/`, `%`
- Unary: `+`, `-`, `~`, `!`
- `defined(X)` and `defined X`
- Integer literals
- Character literals (converted to their integer value)
- Parenthesized sub-expressions
- Ternary: `? :`

---

## Acceptance Criteria

### Core Functionality
- [ ] Object-like macros expand correctly
- [ ] Function-like macros expand correctly with argument substitution
- [ ] Variadic macros work (`__VA_ARGS__`)
- [ ] Stringification (`#`) produces correct string literals with proper escaping of `"` and `\` inside string/char literal arguments
- [ ] Token pasting (`##`) creates new tokens and re-lexes them using forge_lexer (not a custom mini-lexer)
- [ ] Recursive expansion prevented (blue-paint/hide-set)
- [ ] `#if`/`#ifdef`/`#ifndef`/`#elif`/`#else`/`#endif` work with arbitrary nesting
- [ ] `#if` constant expressions evaluate correctly including `defined()` and unsigned arithmetic (suffix-aware: `1U`, `1UL`, `1ULL` are unsigned)
- [ ] `#include <...>` resolves system headers
- [ ] `#include "..."` resolves relative to including file
- [ ] `#error` produces compiler error
- [ ] `#line` changes reported line/file
- [ ] `#pragma once` prevents double inclusion
- [ ] Predefined macros produce correct values

### Edge Cases
- [ ] Empty macro: `#define EMPTY` expands to nothing
- [ ] Macro expanding to another macro: `#define A B` `#define B 42` → `A` expands to `42`
- [ ] Self-referential macro: `#define X X` → `X` expands to `X` (once, no infinite loop)
- [ ] Mutual recursion: `#define A B` `#define B A` → `A` → `B` → `A` (stops, no loop)
- [ ] Function-like macro without invocation: `#define F(x) x` then `F` alone (not followed by `(`) is NOT expanded
- [ ] Empty arguments: `F(,)` passes two empty arguments
- [ ] Nested parentheses in arguments: `F((1,2))` passes one argument `(1,2)`
- [ ] `##` with empty argument: `#define PASTE(a,b) a##b` → `PASTE(,x)` produces `x`
- [ ] `__VA_ARGS__` empty: `LOG("hi")` where `LOG(fmt, ...)` — the `, __VA_ARGS__` produces a trailing comma (GNU's `, ##__VA_ARGS__` extension removes it, but we don't need this yet)
- [ ] `#if 0` blocks may contain arbitrary garbage (even unbalanced quotes, as long as the newlines and `#endif` are intact)

### Real-World
- [ ] Can preprocess `#include <stddef.h>` on Ubuntu without errors
- [ ] Can preprocess `#include <stdint.h>` on Ubuntu without errors
- [ ] Can preprocess `#include <limits.h>` on Ubuntu without errors
- [ ] Can preprocess `#include <stdio.h>` on Ubuntu without errors (this is the big test — stdio.h pulls in dozens of other headers)

---

## Claude Code Prompts

### Prompt 2.1 — Preprocessor skeleton, token cursor, and directive dispatch

```
Create the forge_preprocess crate in the Forge workspace.

This crate implements the C17 preprocessor. It takes a Vec<Token> from forge_lexer and produces a new Vec<Token> with all preprocessor directives processed and all macros expanded.

Core infrastructure to build:

1. **TokenCursor** — a wrapper around a Vec<Token> with:
   - peek() -> Option<&Token>
   - advance() -> Option<Token>
   - push_front(tokens: Vec<Token>) — inject tokens at the current position (needed for macro expansion and #include)
   - skip_to_end_of_line() — consume tokens until the next at_start_of_line token or EOF
   - collect_to_end_of_line() -> Vec<Token> — collect tokens until end of line (for directive arguments)

2. **Preprocessor struct** with:
   - macros: HashMap<String, MacroDef>
   - include_paths: Vec<PathBuf>
   - include_stack: Vec<IncludeFrame> (filename + depth, for circular detection)
   - if_stack: Vec<IfState> (for nested #if tracking)
   - diagnostics: Vec<Diagnostic>

3. **MacroDef enum**:
   - ObjectLike { name: String, replacement: Vec<Token>, is_predefined: bool }
   - FunctionLike { name: String, params: Vec<String>, is_variadic: bool, replacement: Vec<Token> }

4. **IfState struct**:
   - any_branch_taken: bool
   - current_branch_active: bool
   - else_seen: bool
   - if_location: Span

5. **Main processing loop** — pub fn preprocess(tokens: Vec<Token>, config: PreprocessConfig) -> Result<Vec<Token>, Vec<Diagnostic>>
   The loop:
   - Read the next token
   - If it's Hash (#) AND at_start_of_line is true:
     - Read the next token (directive name)
     - Dispatch to handler: handle_define, handle_undef, handle_if, handle_ifdef, handle_ifndef, handle_elif, handle_else, handle_endif, handle_include, handle_error, handle_warning, handle_line, handle_pragma
     - If inside a false conditional block (if_stack.last().current_branch_active == false), only process conditional directives (#if, #ifdef, #ifndef, #elif, #else, #endif) and skip everything else
   - If NOT a directive: if in an active conditional block, try macro expansion, then emit to output
   - If in an inactive conditional block, skip the token

6. **PreprocessConfig struct**:
   - include_paths: Vec<PathBuf>
   - target_arch: TargetArch (X86_64 or AArch64)
   - predefined_macros: Vec<(String, String)>

7. **handle_define** (basic version for now):
   - Read macro name (must be identifier)
   - If next token is LeftParen AND has_leading_space is FALSE → function-like macro
     - Parse parameter list (identifiers, optional ... for variadic)
     - Collect replacement tokens to end of line
   - Otherwise → object-like macro
     - Collect replacement tokens to end of line
   - Store in macros HashMap
   - If name already defined: warn if redefinition differs

8. **handle_undef**:
   - Read macro name
   - Remove from macros HashMap

9. Do NOT implement macro expansion yet — just store the definitions.
   Do NOT implement #include yet.
   Do NOT implement conditional compilation yet.
   Just the skeleton, #define, #undef, and the processing loop.

Write tests:
- Define an object-like macro, verify it's stored
- Define a function-like macro (verify the has_leading_space distinction)
- #undef removes a macro
- Redefining a macro with same replacement: no warning
- Redefining a macro with different replacement: warning diagnostic
- Non-directive tokens pass through unchanged
- Hash token NOT at start of line passes through as a regular token

Add forge_preprocess to workspace Cargo.toml and as a dependency of forge_driver.
```

### Prompt 2.2 — Object-like macro expansion with hide-set

```
Implement object-like macro expansion in forge_preprocess.

IMPORTANT DESIGN DECISION: Tokens in the preprocessor need a "hide set" — the set of macro names that cannot be expanded from this token. This prevents infinite recursion.

Option A: Add a hide_set: HashSet<String> field directly to forge_lexer::Token.
Option B: Create a wrapper type in forge_preprocess:

```rust
struct PPToken {
    token: Token,
    hide_set: HashSet<String>,
}
```

Use Option B (wrapper) so we don't pollute the lexer with preprocessor concerns. The preprocessor internally works with PPToken, and strips the hide_set when producing the final output.

Implement:

1. **PPToken wrapper** with hide_set field and convenience methods to access the inner Token fields.

2. **Object-like expansion**:
   - When processing a non-directive identifier token, check if it's a defined object-like macro
   - Check: is the macro name in this token's hide_set? If yes, skip expansion (emit as-is)
   - Take the replacement token list from the macro definition
   - Clone the replacement tokens, add the macro name to each token's hide_set
   - Also UNION the current token's hide_set into each replacement token's hide_set
   - Insert the replacement tokens back into the token cursor for rescanning
   - The rescan will pick up further macro expansions (but the hide_set prevents re-expanding the same macro)

3. **Rescanning**: after substitution, the replaced tokens are rescanned from left to right. This means if macro A expands to `B + C` and B is also a macro, B will be expanded on the rescan pass.

4. **Self-referential prevention**: 
   - `#define X X` → when expanding X, the replacement is the token `X` with `{"X"}` in its hide_set → on rescan, X is in the hide_set, so it's not expanded again → output is `X`
   - `#define A B` + `#define B A` → expanding A produces `B` with hide_set `{"A"}`. Rescanning expands B (not in hide_set) to `A` with hide_set `{"A", "B"}`. Rescanning: A is in hide_set, stop. Output: `A`.

Write tests:
- Simple expansion: `#define FOO 42` → `FOO` expands to `42`
- Chain expansion: `#define A B` + `#define B 42` → `A` expands to `42`
- Self-referential: `#define X X` → `X` stays as `X`
- Mutual recursion: `#define A B` + `#define B A` → `A` becomes `A` (not infinite)
- Multiple tokens: `#define FOO 1 + 2` → `FOO` expands to `1 + 2`
- Empty macro: `#define EMPTY` → `EMPTY` expands to nothing
- Expansion in context: `int x = FOO;` → `int x = 42;`
- Macro not followed by replacement: `#define FLAG` → `#ifdef FLAG` would work (tested later)
- hide_set propagation: `#define A A_REAL` + `#define A_REAL A` → verify terminates
```

### Prompt 2.3 — Function-like macro expansion (argument collection and substitution)

```
Extend forge_preprocess with function-like macro expansion.

This is the most complex part of the preprocessor. Implement it carefully.

1. **Detecting function-like invocation**:
   - When an identifier matches a function-like macro name, check if the NEXT token is LeftParen
   - If no LeftParen follows, do NOT expand — the identifier passes through unchanged
   - This means: `#define F(x) x*x` then `int F = 5;` → `F` is NOT expanded (no parens)

2. **Argument collection** — collect_macro_arguments():
   - Read tokens between the outer `(` and matching `)`
   - Split on commas, BUT respect nested parentheses: `F((a,b), c)` has TWO arguments: `(a,b)` and `c`
   - For variadic macros: if the macro has N params + `...`, the first N-1 commas split normally, all remaining tokens (including commas) go into the variadic argument
   - Handle empty arguments: `F(,)` → two empty argument lists, `F()` → one empty argument if macro has one param, or zero args if macro has zero params
   - Trim leading/trailing whitespace from each argument (but preserve internal whitespace)

3. **Argument substitution** — substitute_args():
   Walk the replacement token list. For each token:
   
   a. If the token is `#` followed by a parameter name → STRINGIFY (C17 §6.10.3.2)
      - Take the RAW (unexpanded) argument tokens
      - Convert them to a string: concatenate their spellings with single spaces between tokens that had has_leading_space
      - ESCAPING (critical): when converting to the string, any `"` or `\` character that appears INSIDE a string literal or character literal argument must be escaped with a preceding `\`. Example:
        - `#define STR(x) #x` → `STR("hello\n")` → `"\"hello\\n\""` (the inner quotes become `\"`, the inner backslash becomes `\\`)
        - `#define STR(x) #x` → `STR('a')` → `"'a'"` (single quotes don't need escaping)
        - `#define STR(x) #x` → `STR(a "b" c)` → `"a \"b\" c"` (embedded quotes in argument)
      - Leading and trailing whitespace of the argument is trimmed
      - Produce a StringLiteral token with the resulting content
   
   b. If the token is adjacent to `##` (either left or right of `##`) AND is a parameter name → use RAW (unexpanded) argument tokens (we'll handle the actual pasting in step 4)
   
   c. Otherwise, if the token is a parameter name → EXPAND the argument first, then substitute
      - "Expand the argument" means: take the argument's token list and run macro expansion on it
      - Cache expanded arguments so we don't re-expand the same argument multiple times
   
   d. If the token is `__VA_ARGS__` in a variadic macro → replace with the variadic arguments (expanded in normal context, raw in ## context)

4. **Token pasting** — process_paste():
   After argument substitution, walk the replacement list looking for `##`:
   - Take the token to the left and right of `##`
   - Concatenate their spellings (the text they represent)
   - Re-lex the concatenated text to produce a new token (it could be a different token type)
   - IMPORTANT: You need a way to lex a short string fragment. Check if forge_lexer exposes a function like `lex(source: &str) -> Vec<Token>` that works on arbitrary string snippets (not just whole files). If the current lexer API requires a filename or has global state assumptions, add a `pub fn lex_fragment(input: &str) -> Vec<Token>` utility function to forge_lexer that lexes a small piece of text with no file context. Do NOT write a mini-lexer inside the preprocessor — reuse forge_lexer.
   - If re-lexing produces zero tokens (empty paste), that's valid
   - If re-lexing produces more than one token, that's undefined behavior (emit warning)
   - The new token's hide_set is the INTERSECTION of the two original tokens' hide_sets
   - Remove the `##` and the two operands, insert the new token(s)

5. After substitution and pasting, add the macro name to every resulting token's hide_set (same as object-like), then feed back into the token cursor for rescanning.

Write THOROUGH tests:
- Simple function-like: `#define SQUARE(x) x * x` → `SQUARE(5)` → `5 * 5`
- Multi-param: `#define ADD(a, b) a + b` → `ADD(1, 2)` → `1 + 2`
- Nested parens in arg: `#define F(x) x` → `F((1, 2))` → `(1, 2)`
- Empty argument: `#define F(x) [x]` → `F()` → `[]`
- Argument used twice: `#define DOUBLE(x) x + x` → `DOUBLE(3)` → `3 + 3`
- Argument with side effects: `#define F(x) x + x` → `F(a + b)` → `a + b + a + b` (NOT `a + b + a + b` with extra parens — the preprocessor doesn't add parens)
- Expansion of arguments: `#define ID(x) x` + `#define A 42` → `ID(A)` → `42` (A is expanded before substitution)
- Stringification: `#define STR(x) #x` → `STR(hello)` → `"hello"`
- Stringification with spaces: `#define STR(x) #x` → `STR(a + b)` → `"a + b"`
- Stringification preserves argument literally: `#define STR(x) #x` + `#define A 42` → `STR(A)` → `"A"` (NOT `"42"` — stringify uses raw arg)
- Stringification ESCAPING: `#define STR(x) #x` → `STR("hello")` → `"\"hello\""` (inner quotes escaped)
- Stringification ESCAPING backslash: `#define STR(x) #x` → `STR("a\nb")` → `"\"a\\nb\""` (inner backslash escaped)
- Stringification ESCAPING char literal: `#define STR(x) #x` → `STR('\\')` → `"'\\\\'"` (backslash in char literal doubled)
- Token paste: `#define PASTE(a, b) a##b` → `PASTE(foo, bar)` → `foobar` (identifier)
- Token paste creating number: `#define PASTE(a, b) a##b` → `PASTE(1, 2)` → `12` (integer literal)
- Token paste with parameter: `#define PREFIX(name) my_##name` → `PREFIX(var)` → `my_var`
- Paste uses raw arg: `#define PASTE(a, b) a##b` + `#define X y` → `PASTE(X, z)` → `Xz` (NOT `yz`)
- Variadic: `#define LOG(fmt, ...) printf(fmt, __VA_ARGS__)` → `LOG("x=%d", x)` → `printf("x=%d", x)`
- Macro not invoked (no parens): `#define F(x) x` → `int F;` → `int F;`
- Nested macro calls: `#define OUTER(x) INNER(x)` + `#define INNER(x) x+1` → `OUTER(5)` → `5+1`
```

### Prompt 2.4 — Conditional compilation (#if, #ifdef, etc.)

```
Implement conditional compilation in forge_preprocess.

This requires two things: (1) tracking active/inactive blocks, and (2) evaluating #if constant expressions.

Part A — Conditional block tracking:

1. **#ifdef IDENT**: push IfState. If IDENT is defined, current_branch_active = true.
2. **#ifndef IDENT**: push IfState. If IDENT is NOT defined, current_branch_active = true.
3. **#if EXPR**: push IfState. Evaluate expression; if non-zero, current_branch_active = true.
4. **#elif EXPR**: 
   - Must have matching #if/#ifdef/#ifndef on the stack
   - If else_seen, ERROR: #elif after #else
   - If any_branch_taken is already true, set current_branch_active = false
   - Otherwise, evaluate expression; if non-zero, set current_branch_active = true, any_branch_taken = true
5. **#else**:
   - Must have matching #if on stack
   - If else_seen, ERROR: duplicate #else
   - Set else_seen = true
   - current_branch_active = !any_branch_taken
6. **#endif**:
   - Pop the if_stack
   - If stack is empty, ERROR: unmatched #endif

CRITICAL: When inside an INACTIVE block, the preprocessor must still:
- Track nested #if/#endif to maintain correct stack depth
- Skip everything else (don't expand macros, don't process #include, etc.)
- But: the skipped tokens may contain unbalanced quotes, invalid syntax, etc. — that's legal! `#if 0 ... arbitrary junk ... #endif` is valid C.

For skipping in inactive blocks:
- Look for # at start of line
- Read the directive name
- Only respond to: if, ifdef, ifndef (push), elif, else (toggle), endif (pop)
- Skip everything else silently

Part B — Constant expression evaluator:

Create a CondExprEvaluator that processes the tokens after #if / #elif:

1. FIRST, handle `defined`:
   - Before macro-expanding the expression, find all `defined IDENT` and `defined(IDENT)` occurrences
   - Replace each with IntegerLiteral 1 (if defined) or 0 (if not)
   - This must happen BEFORE macro expansion, because defined(FOO) must check if FOO is defined, not expand FOO first

2. THEN, macro-expand the remaining tokens in the expression

3. THEN, replace any remaining identifiers with IntegerLiteral 0 (per C standard §6.10.1/4)

4. THEN, parse and evaluate the integer constant expression using a small Pratt parser:
   - Track signedness: each value in the evaluator should be represented as a tagged type:
     ```rust
     enum PPValue {
         Signed(i64),    // intmax_t
         Unsigned(u64),  // uintmax_t
     }
     ```
   - Integer literal suffix determines signedness: `1` is Signed, `1U` or `1UL` or `1ULL` is Unsigned
   - When mixing signed and unsigned in a binary operation, apply C's usual arithmetic conversions: if either operand is unsigned, convert both to unsigned (this matches C17 §6.10.1/4)
   - This matters for real headers: `#if -1 < 1U` evaluates to FALSE in C because -1 is converted to unsigned (becomes UINTMAX_MAX), and UINTMAX_MAX is NOT less than 1
   - If you only use i64, `-1 < 1` would wrongly evaluate to TRUE
   - Support operators: + - * / % << >> & | ^ ~ ! && || == != < > <= >= ? : ( )
   - Character literals are valid: 'A' evaluates to Signed(65)
   - Division by zero: emit warning and treat as Signed(0)
   - Overflow: wrap silently (both signed and unsigned, per C preprocessor rules)

Write tests:
- `#ifdef FOO` when FOO is defined → active
- `#ifdef FOO` when FOO is not defined → inactive
- `#ifndef FOO` → opposite
- `#if 1` → active, `#if 0` → inactive
- `#if 1 + 1` → active (evaluates to 2, which is true)
- `#if defined(FOO)` when FOO is defined → active
- `#if defined FOO` (without parens) → works
- `#if defined(FOO) && defined(BAR)` → both must be defined
- `#if FOO == 42` where `#define FOO 42` → active (FOO expands to 42)
- `#elif` chain: first true branch wins, later branches inactive
- Nested: `#if 1 ... #if 0 ... #endif ... #endif`
- `#if 0` skipping code with unbalanced content: `#if 0\n "unterminated string\n #endif` → no error
- Error: `#else` without `#if`
- Error: `#endif` without `#if`
- Error: `#elif` after `#else`
- Error: unterminated `#if` at end of file
- `#if 'A' == 65` → active (char literal in expression)
- `#if (1 << 4) == 16` → active
- `#if 0 || 1` → active
- Unsigned arithmetic: `#if -1 < 1U` → INACTIVE (0) because -1 promotes to unsigned, becomes UINTMAX_MAX
- Unsigned arithmetic: `#if 0U - 1 > 0` → ACTIVE (1) because unsigned 0-1 wraps to UINTMAX_MAX
- Unsigned suffix: `#if 1ULL + 1ULL == 2` → active
- Complex: `#if (defined(__linux__) && defined(__x86_64__)) || defined(__aarch64__)`
```

### Prompt 2.5 — #include with search paths and recursive preprocessing
```
Implement #include handling, predefined macros, and __has_* builtins in forge_preprocess.
 
This is where the preprocessor becomes a real compiler component — it reads files from disk and recursively processes them. After this prompt, `#include <stdio.h>` must preprocess without errors on the host system.
 
IMPORTANT: Read all 9 sections before writing code. Sections 1–3 (include parsing) and 9 (predefined macros) are tightly coupled — system headers won't work without the predefined macros.
 
────────────────────────────────────────────────────────
SECTION 1 — Parsing the #include argument
────────────────────────────────────────────────────────
 
After seeing `#` `include` at the start of a line, the next tokens determine the include form. There are THREE forms:
 
Form A — #include <filename>
  The lexer does NOT produce a single "header-name" token.
  It tokenizes `<stdio.h>` as separate tokens: `<`, `stdio`, `.`, `h`, `>`.
  You must:
  - See that the next token is LessThan (<)
  - Collect ALL tokens (including their spellings) until you find GreaterThan (>)
  - Concatenate the spellings to form the filename string: "stdio.h"
  - Do NOT include the < and > in the filename
  - If no > is found before end of line, ERROR: "expected '>' in #include"
  - This is a SYSTEM include — search only system paths (step described in Section 2)
 
Form B — #include "filename"
  The lexer produces a StringLiteral token for "filename".
  You must:
  - Extract the text content from the StringLiteral token
  - STRIP the surrounding quote characters — the token's spelling includes the quotes
  - This is a LOCAL include — search relative first, then system paths (Section 2)
 
Form C — #include MACRO_TOKENS (computed include)
  If the token after `include` is neither < nor a StringLiteral:
  - Collect tokens to end of line
  - Macro-expand them
  - After expansion, the result must match Form A (<...>) or Form B ("...")
  - Then process accordingly
  - If after expansion it's still neither form, ERROR
 
────────────────────────────────────────────────────────
SECTION 2 — Include search paths
────────────────────────────────────────────────────────
 
1. #include <filename> (system include):
   - Search each path in include_paths in order
   - The file would be at: {include_path}/{filename}
   - Use std::path::Path::join and check .exists()
   - If not found in any path, ERROR: "'filename' file not found"
 
2. #include "filename" (local include):
   - First, resolve relative to the directory of the CURRENTLY BEING PROCESSED file
     (not the original source file — this matters for nested includes like:
      main.c includes "sub/a.h", and a.h includes "b.h" — b.h should be found in sub/)
   - Get the current file path from the include stack (or the initial source file)
   - Try: {current_file_dir}/{filename}
   - If not found, fall back to system include paths (same as <> search)
   - If not found anywhere, ERROR
 
3. System include path auto-detection — create function detect_system_include_paths():
   - Run: `cc -E -v -x c /dev/null 2>&1` (captures stderr)
   - Parse output: look for the line "#include <...> search starts here:"
   - Collect all lines after it until "End of search list."
   - Each line (trimmed) is a path — filter to only paths that exist on disk
   - Return these as Vec<PathBuf>
   - Fallback if the command fails or produces no paths:
     On Linux: ["/usr/include", "/usr/local/include", "/usr/include/x86_64-linux-gnu"]
     On macOS: ["/usr/local/include", "/usr/include"]
     (Use std::env::consts::OS to detect)
   - Call this function ONCE during Preprocessor initialization and cache the result
 
────────────────────────────────────────────────────────
SECTION 3 — Recursive include processing
────────────────────────────────────────────────────────
 
Once the file path is resolved:
 
1. Check #pragma once cache: if this file's canonical path (std::fs::canonicalize) is in the
   pragma_once_files set, SKIP entirely — emit no tokens, return immediately
 
2. Check include guard cache: if this file's canonical path is in the include_guard_files map
   AND the guard macro is still defined, SKIP entirely
 
3. Check circular includes: if this file is already on the include_stack, ERROR:
   "circular include: file.h → ... → file.h"
   (Include the chain in the error message for debuggability)
 
4. Check depth limit: if include_stack.len() >= 200, ERROR:
   "include depth limit exceeded (200 levels)"
 
5. Read the file contents (std::fs::read_to_string)
   - If read fails, ERROR with the OS error message
 
6. Lex the file using forge_lexer (lex the full file to Vec<Token>)
 
7. Push a frame onto include_stack with the filename
 
8. Save the current __FILE__ value, set __FILE__ to the new filename
 
9. Recursively preprocess the tokens:
   - The macro table is SHARED (macros defined in the included file persist after the include)
   - The if_stack should be SEPARATE for the included file (or assert it's balanced at end)
   - The output tokens from the included file are spliced into the current output
 
10. Pop the include_stack frame
 
11. Restore the previous __FILE__ value
 
────────────────────────────────────────────────────────
SECTION 4 — #pragma once
────────────────────────────────────────────────────────
 
When `#pragma once` is encountered:
- Get the canonical path of the current file (the file being processed)
- Add it to a HashSet<PathBuf> called pragma_once_files on the Preprocessor struct
- Before processing any #include, check this set (as described in Section 3)
 
────────────────────────────────────────────────────────
SECTION 5 — Include guard detection (REQUIRED, not optional)
────────────────────────────────────────────────────────
 
System headers use include guards, not #pragma once. Without detecting these,
including <stdio.h> will re-process the same headers repeatedly, causing:
- Duplicate definitions → errors
- Massive performance degradation
- Potential infinite loops through diamond includes
 
Detection algorithm — after preprocessing an included file, check:
- Did the file start with #ifndef GUARD_MACRO / #define GUARD_MACRO (as first two directives)?
- Did the file end with #endif as the last directive?
- Was there no code outside this #ifndef...#endif block?
 
If all three: record the mapping canonical_path → GUARD_MACRO in include_guard_files.
On subsequent includes, if the guard macro is still defined, skip the file.
 
A simpler approximation (acceptable for now): when you see the pattern
  #ifndef X
  #define X
at the very start of a file (first two directives), record X as a potential guard for that file.
At the end of that file's processing, if the last directive was #endif and it matched that
outermost #ifndef, confirm it as a guard.
 
────────────────────────────────────────────────────────
SECTION 6 — Predefined macros
────────────────────────────────────────────────────────
 
Set up these macros BEFORE preprocessing begins. Add them in a setup_predefined_macros() method.
 
Standard C macros:
  __STDC__            → 1
  __STDC_VERSION__    → 201710L
  __STDC_HOSTED__     → 1
 
Magic macros (evaluated at point of use, NOT stored as fixed replacement):
  __FILE__   → StringLiteral with the current filename (changes per #include)
  __LINE__   → IntegerLiteral with the current line number
  __DATE__   → StringLiteral "Mmm dd yyyy" (set ONCE at preprocessor start, use chrono or manual formatting)
  __TIME__   → StringLiteral "hh:mm:ss" (set ONCE at preprocessor start)
 
Implement __FILE__ and __LINE__ as special cases in the macro expansion path:
  - When expanding __FILE__, generate a fresh StringLiteral token with the current file name
  - When expanding __LINE__, generate a fresh IntegerLiteral token with the current line number
  - Do NOT store fixed replacement lists for these — they must be dynamic
 
Platform macros (detect at runtime using cfg!() or std::env::consts):
  __linux__             → 1   (if on Linux)
  __unix__              → 1   (if on Unix-like)
  __APPLE__             → 1   (if on macOS)
  __x86_64__            → 1   (if on x86_64)
  __aarch64__           → 1   (if on AArch64/ARM64)
  __LP64__              → 1   (if 64-bit pointers)
  __BYTE_ORDER__        → 1234  (little-endian; 4321 for big)
  __ORDER_LITTLE_ENDIAN__ → 1234
  __ORDER_BIG_ENDIAN__    → 4321
 
GCC compatibility macros (CRITICAL — system headers check these):
  __GNUC__              → 14
  __GNUC_MINOR__        → 0
  __GNUC_PATCHLEVEL__   → 0
  __GNUC_STDC_INLINE__  → 1
 
Size/type macros (needed by <stdint.h> and <limits.h>):
  __SIZEOF_SHORT__      → 2
  __SIZEOF_INT__        → 4
  __SIZEOF_LONG__       → 8   (LP64)
  __SIZEOF_LONG_LONG__  → 8
  __SIZEOF_POINTER__    → 8
  __SIZEOF_FLOAT__      → 4
  __SIZEOF_DOUBLE__     → 8
  __SIZEOF_SIZE_T__     → 8
  __SIZEOF_WCHAR_T__    → 4
  __SIZEOF_PTRDIFF_T__  → 8
 
  __INT8_TYPE__         → signed char
  __INT16_TYPE__        → short
  __INT32_TYPE__        → int
  __INT64_TYPE__        → long int
  __UINT8_TYPE__        → unsigned char
  __UINT16_TYPE__       → unsigned short
  __UINT32_TYPE__       → unsigned int
  __UINT64_TYPE__       → long unsigned int
  __INTPTR_TYPE__       → long int
  __UINTPTR_TYPE__      → long unsigned int
  __SIZE_TYPE__         → long unsigned int
  __PTRDIFF_TYPE__      → long int
  __WCHAR_TYPE__        → int
 
  __INT_MAX__           → 2147483647
  __LONG_MAX__          → 9223372036854775807L
  __LONG_LONG_MAX__     → 9223372036854775807LL
  __SHRT_MAX__          → 32767
  __SCHAR_MAX__         → 127
  __WCHAR_MAX__         → 2147483647
 
  __INT8_MAX__          → 127
  __INT16_MAX__         → 32767
  __INT32_MAX__         → 2147483647
  __INT64_MAX__         → 9223372036854775807L
  __UINT8_MAX__         → 255
  __UINT16_MAX__        → 65535
  __UINT32_MAX__        → 4294967295U
  __UINT64_MAX__        → 18446744073709551615UL
 
  __FLT_MIN__           → 1.17549435e-38F
  __FLT_MAX__           → 3.40282347e+38F
  __DBL_MIN__           → 2.2250738585072014e-308
  __DBL_MAX__           → 1.7976931348623157e+308
  __FLT_EPSILON__       → 1.19209290e-7F
  __DBL_EPSILON__       → 2.2204460492503131e-16
 
NOTE: For the __*_TYPE__ macros, the replacement is MULTIPLE tokens (e.g., "long unsigned int"
is three tokens). Create the replacement token lists properly — lex the replacement string
using lex_fragment() to produce correct tokens.
 
────────────────────────────────────────────────────────
SECTION 7 — __has_include, __has_builtin, __has_attribute, __has_feature
────────────────────────────────────────────────────────
 
Modern system headers (especially glibc's <features.h> and Clang-compatible headers) use these
extensively. They look like function-like macros but have special behavior.
 
Implement as special-cased function-like macros:
 
1. __has_include(<header>) and __has_include("header"):
   - Actually check if the file exists using the same search path logic as #include
   - Return 1 if found, 0 if not
   - The argument is parsed the same way as #include (Form A or Form B)
   - This appears inside #if expressions: `#if __has_include(<stdatomic.h>)`
 
2. __has_builtin(X):
   - For now: always expand to 0
   - (We don't support any GCC builtins yet at the preprocessor level)
 
3. __has_attribute(X):
   - For now: always expand to 0
 
4. __has_feature(X), __has_extension(X):
   - For now: always expand to 0
 
5. __has_warning(X):
   - For now: always expand to 0
 
6. __has_c_attribute(X):
   - For now: always expand to 0
 
Implementation approach: when the #if expression evaluator encounters these identifiers
followed by `(`, treat them as special built-in operators (similar to `defined()`),
parse their argument, and evaluate. They should NOT be stored in the regular macro table —
handle them in the expression evaluator.
 
Alternative simpler approach: define them as regular function-like macros that expand to 0,
EXCEPT for __has_include which needs real file-existence checks. This is simpler but
__has_include won't work in #if expressions properly. Choose whichever approach you think
is cleaner, but __has_include MUST work in #if directives.
 
────────────────────────────────────────────────────────
SECTION 8 — Additional GCC/Clang compatibility tokens
────────────────────────────────────────────────────────
 
System headers will produce tokens that the preprocessor doesn't need to understand —
it just passes them through to the parser. But the preprocessor must NOT choke on them:
 
- __attribute__((...))  → pass through all tokens as-is
- __extension__         → pass through as a regular identifier token
- __restrict            → pass through as identifier
- __inline              → pass through as identifier
- __volatile__          → pass through as identifier
- __asm__               → pass through as identifier
- __typeof__            → pass through as identifier
- __builtin_va_list     → pass through as identifier (it's used as a type)
- __signed__            → pass through as identifier
- __const               → pass through as identifier
 
The preprocessor's job is macro expansion and directive processing.
Any identifier it doesn't recognize as a macro just passes through unchanged.
So these should work automatically — but verify with a test that tokens like
__attribute__ pass through without errors.
 
────────────────────────────────────────────────────────
SECTION 9 — Tests
────────────────────────────────────────────────────────
 
Write tests in this order (each one builds on the previous):
 
A. Local include basics (use tempdir crate or std::env::temp_dir):
   - Create temp dir with test.c and header.h
     header.h: `#define VALUE 42\n`
     test.c:   `#include "header.h"\nint x = VALUE;\n`
   - Preprocess test.c → output should contain tokens for `int x = 42 ;`
 
B. Nested includes:
   - test.c includes "a.h", a.h includes "b.h", b.h defines a macro
   - Verify the macro is visible in test.c after preprocessing
 
C. Relative path resolution:
   - test.c includes "sub/a.h", sub/a.h includes "b.h"
   - b.h should be found in sub/ (relative to a.h, not to test.c)
 
D. #pragma once:
   - header.h has #pragma once and defines COUNTER as some value
   - test.c includes header.h twice
   - Verify no duplicate definition errors
 
E. Include guard:
   - header.h uses #ifndef/#define/#endif guard pattern
   - Include it twice, verify it's only processed once
 
F. Circular include detection:
   - a.h includes b.h, b.h includes a.h
   - Verify an error diagnostic is produced (not a stack overflow!)
 
G. File not found:
   - #include "nonexistent.h" → error with clear message
   - #include <nonexistent.h> → error with clear message
 
H. __FILE__ and __LINE__:
   - Verify __FILE__ produces the current filename
   - Verify __LINE__ produces the correct line number
   - Verify __FILE__ changes inside an included file and restores after
 
I. __STDC_VERSION__:
   - `#if __STDC_VERSION__ == 201710L` → should be active
 
J. Predefined platform macros:
   - `#ifdef __linux__` or `#ifdef __APPLE__` → at least one should be active
   - `#ifdef __GNUC__` → should be active (we define it)
 
K. __has_include:
   - `#if __has_include(<stddef.h>)` → should be true (1)
   - `#if __has_include("nonexistent.h")` → should be false (0)
 
L. System header smoke tests (THE BOSS FIGHT):
   In order, preprocess a file containing ONLY:
   1. `#include <stddef.h>`    → assert zero errors
   2. `#include <stdint.h>`    → assert zero errors
   3. `#include <limits.h>`    → assert zero errors
   4. `#include <stdio.h>`     → assert zero errors
 
   For each: create a real preprocessing test that reads from the actual system headers.
   If ANY of these fail, investigate and fix. Common blockers:
   - Missing predefined macro → add it
   - __has_include not working in #if → fix expression evaluator
   - Unknown directive or token → the preprocessor should pass it through, not error
   - Include guard not detected → files re-included → duplicate macro definitions
 
   NOTE: If on macOS, the system headers are in a different location (Xcode SDK).
   The detect_system_include_paths() function should handle this. If tests fail on
   macOS but the logic is correct, add a #[cfg(target_os = "linux")] gate on the
   stdio.h test and add a NOTE comment explaining why.
 
M. Computed include:
   - `#define HEADER "header.h"` then `#include HEADER` → should work
 
Run `cargo test --all` at the end to verify all existing 400+ tests still pass.
Run `cargo clippy --all-targets -- -D warnings` — must be clean.
```
 
---

### Prompt 2.6 — #error, #warning, #line, #pragma, _Pragma

```
Implement the remaining preprocessor directives: #error, #warning, #line, #pragma, _Pragma,
and the null directive. After this prompt, all C17 preprocessor directives are complete.
 
IMPORTANT: _Pragma is the trickiest part here because it's NOT a directive — it can appear
anywhere in the token stream, including as the result of macro expansion. Read all 7 sections
before writing code.
 
────────────────────────────────────────────────────────
SECTION 1 — Null directive (fix first if needed)
────────────────────────────────────────────────────────
 
A line containing just `#` (possibly followed by whitespace) is a valid "null directive."
It does nothing.
 
This is important because:
- Some system headers have blank `#` lines
- Code generators produce them
- Your main processing loop must handle this: when you see `#` at_start_of_line and the
  next token is ALSO at_start_of_line (or EOF), it's a null directive — just continue
 
If this already works, great. If it currently errors with "unknown directive", fix it.
 
Write a test: preprocess `#\nint x = 1;\n` → output should be `int x = 1 ;`
 
────────────────────────────────────────────────────────
SECTION 2 — #error
────────────────────────────────────────────────────────
 
Syntax: `#error message tokens until end of line`
 
Behavior:
- Collect all tokens from after `error` to end of line
- Concatenate their spellings (respecting has_leading_space for spaces between) to form
  the human-readable message
- Emit a Diagnostic::error with:
  - message: the concatenated text
  - span: the span of the `#error` directive (the `#` token's span)
- Set a `has_errors` flag on the Preprocessor (so the final result reports failure)
- But DO NOT stop processing — continue preprocessing the rest of the file
  (this matches GCC/Clang behavior: report the error, continue to find more errors)
 
CRITICAL: #error inside an inactive conditional block (#if 0) must NOT fire.
This is already handled if your inactive-block skipping from Prompt 2.4 works correctly,
but write an explicit test for it.
 
────────────────────────────────────────────────────────
SECTION 3 — #warning (GNU extension)
────────────────────────────────────────────────────────
 
Syntax: `#warning message tokens until end of line`
 
Identical to #error except:
- Emit Diagnostic::warning instead of Diagnostic::error
- Does NOT set has_errors — it's a warning, not an error
- Processing continues normally
 
#warning is not in the C17 standard but is a GNU extension used by essentially all
real-world C code. Treat it as a first-class directive.
 
Also inside #if 0: must NOT fire (same as #error).
 
────────────────────────────────────────────────────────
SECTION 4 — #line
────────────────────────────────────────────────────────
 
Syntax: `#line number` or `#line number "filename"`
 
Behavior:
- FIRST, macro-expand the tokens after `line` (this is required by C17 — 
  `#line __LINE__` is valid)
- After expansion, parse:
  - number: must be a positive integer literal, 1 ≤ N ≤ 2147483647
  - optional "filename": a string literal
- If number is invalid (0, negative, too large, or not a number): 
  ERROR: "invalid line number in #line directive"
- Effect:
  - Set the "line offset" for the current file context such that the NEXT line
    has the specified line number
  - If filename is provided, set the "file override" for the current file context
- These overrides affect:
  - __LINE__ macro expansion
  - __FILE__ macro expansion
  - Span/location info in diagnostics
- IMPORTANT: these overrides are LOCAL to the current file context.
  When an #include finishes and the include stack is popped, the parent file's
  line/file tracking is restored — the #line effect does NOT leak up.
 
Implementation approach:
- Add `line_offset: Option<(u32, u32)>` to the Preprocessor or to IncludeFrame
  (stores the difference between actual and reported line numbers)
- Add `file_override: Option<String>` similarly
- When computing __LINE__, apply the offset if present
- When computing __FILE__, use the override if present
 
────────────────────────────────────────────────────────
SECTION 5 — #pragma
────────────────────────────────────────────────────────
 
Syntax: `#pragma tokens until end of line`
 
Behavior:
- #pragma once: ALREADY IMPLEMENTED in Prompt 2.5 — keep existing implementation
- #pragma message("text"): emit the text as a Diagnostic::note
- #pragma GCC diagnostic push/pop/ignored/warning/error: silently ignore
  (these are very common in system headers — they control GCC warning behavior,
   which we don't support yet)
- #pragma GCC visibility push/pop: silently ignore (controls ELF symbol visibility)
- #pragma pack(...): silently ignore (controls struct layout — relevant for Phase 4)
- #pragma STDC ...: silently ignore (C standard pragmas like FP_CONTRACT)
- ALL other unknown pragmas: silently ignore (per C17 §6.10.6, unknown pragmas
  cause implementation-defined behavior — ignoring is valid)
 
DO NOT emit warnings for unknown pragmas. Many build systems and libraries use
custom pragmas, and warning on every one would be noisy.
 
────────────────────────────────────────────────────────
SECTION 6 — _Pragma operator
────────────────────────────────────────────────────────
 
This is the hardest part. _Pragma is NOT a directive — it's an OPERATOR that can appear
anywhere in the token stream, including as the result of macro expansion.
 
Example in real system headers (glibc):
```c
#define __THROW __attribute__((__nothrow__))
#define __BEGIN_DECLS _Pragma("GCC visibility push(default)")
```

### Prompt 2.7 — Integration, driver wiring, and full validation

```
Integrate the preprocessor into the Forge compilation pipeline. After this prompt,
`forge -E file.c` produces preprocessed C source on stdout (like gcc -E).
 
────────────────────────────────────────────────────────
SECTION 1 — CLI flags
────────────────────────────────────────────────────────
 
Add these flags to forge_cli (clap-based):
 
1. `-E` flag:
   - When present, run only lexer + preprocessor, then output reconstructed C source
     to STDOUT and exit (do not parse, do not compile)
   - This is the standard flag used by GCC and Clang
 
2. `-I <path>` flag (repeatable):
   - Add path to the include search list
   - Multiple -I flags are searched in ORDER (first -I wins)
   - These paths are searched BEFORE the auto-detected system include paths
   - Example: `forge -E -I ./include -I ../lib/include file.c`
 
3. `-D <macro>[=<value>]` flag (repeatable):
   - `-D FOO` → define FOO as `1` (object-like macro with replacement `1`)
   - `-D FOO=bar` → define FOO as `bar`
   - `-D FOO=` → define FOO as empty (object-like macro with empty replacement)
   - `-D FOO=1+2` → define FOO as `1+2` (the value is the entire string after `=`)
   - `-D 'FOO(x)=x*x'` → define function-like macro (if the name contains `(`)
   - Implementation: split on first `=`. If no `=`, value is "1".
     Then lex the value using lex_fragment() to produce the replacement token list.
     For function-like: if the name part contains `(`, parse parameter list from it.
     Add the definition to PreprocessConfig::predefined_macros BEFORE preprocessing starts.
 
4. `-U <macro>` flag (repeatable):
   - Undefine a macro (removes it from the predefined set)
   - Processed AFTER -D flags, so `-D FOO -U FOO` results in FOO being undefined
   - Useful for overriding default predefined macros:
     `forge -E -U __GNUC__ file.c` would remove the GCC compat macro
 
5. Flag processing order in the driver:
   a. Auto-detect system include paths (detect_system_include_paths)
   b. Process -I flags (prepend to include paths, maintaining order)
   c. Set up predefined macros (setup_predefined_macros)
   d. Process -D flags (define/override macros)
   e. Process -U flags (undefine macros)
   f. Lex the source file
   g. Run preprocessor
   h. If -E: output reconstructed source. Otherwise: continue to parser (Phase 3)
 
────────────────────────────────────────────────────────
SECTION 2 — Update forge_driver pipeline
────────────────────────────────────────────────────────
 
The driver currently runs the lexer. Now wire in the preprocessor:
 
1. After lexing, create a PreprocessConfig with:
   - include_paths from -I flags + auto-detected system paths
   - predefined macros + -D/-U modifications
   - source file path (needed for #include "..." relative resolution)
 
2. Run the preprocessor on the lexed tokens
 
3. If -E flag: reconstruct and print source (Section 3), then exit
4. If `forge check`: for now, just report "preprocessing successful, N tokens"
   (the parser will be wired in Phase 3)
 
5. Propagate preprocessor diagnostics to the main diagnostic output
 
────────────────────────────────────────────────────────
SECTION 3 — Preprocessed source reconstruction (token_stream_to_source)
────────────────────────────────────────────────────────
 
Create a function: pub fn tokens_to_source(tokens: &[Token]) -> String
 
This reconstructs valid, readable C source from the preprocessed token stream.
The output must be parseable C — it's what `gcc -E` produces.
 
Rules:
1. For each token, output its spelling (the text it represents)
 
2. Spacing:
   - If token has has_leading_space == true, output a single space before it
   - If token has has_leading_space == false and is NOT at start of line,
     output no space (tokens are adjacent)
 
3. Newlines:
   - If token has at_start_of_line == true, output a newline before it
     (unless it's the very first token)
 
4. Linemarkers (important for debuggability):
   - When the source file changes (tokens from a different file due to #include),
     output a linemarker: `# <line> "<filename>"`
   - When returning from an include, output the parent's linemarker
   - This matches GCC -E output format
   - Implementation: track the "current file" and "current line" as you walk the tokens.
     When a token's source file differs from current, emit a linemarker.
     When the line number jumps (non-consecutive), emit a linemarker or blank lines.
   
   NOTE: If linemarkers are complex to implement right now, it's acceptable to skip them
   for this prompt and just output a flat token stream. The parser reads the token stream
   directly (not the -E text output), so linemarkers are for human debugging only.
   But at minimum, output newlines between logical lines.
 
5. Special tokens:
   - StringLiteral: output with surrounding quotes: "content"
   - CharLiteral: output with surrounding quotes: 'c'
   - IntegerLiteral, FloatLiteral: output the original text
   - All others: output the canonical spelling
 
6. Output to stdout using print!/println! (not to a file)
 
────────────────────────────────────────────────────────
SECTION 4 — End-to-end lit tests
────────────────────────────────────────────────────────
 
Create test files in tests/lit/preprocess/ that test the FULL pipeline
(CLI → lexer → preprocessor → output):
 
a. tests/lit/preprocess/object_macros.c:
   ```c
   #define WIDTH 80
   #define HEIGHT 24
   int area = WIDTH * HEIGHT;
   // CHECK: int area = 80 * 24 ;
   ```
 
b. tests/lit/preprocess/function_macros.c:
   ```c
   #define MAX(a, b) ((a) > (b) ? (a) : (b))
   int x = MAX(3, 5);
   // CHECK: int x = ((3) > (5) ? (3) : (5)) ;
   ```
 
c. tests/lit/preprocess/stringify.c:
   ```c
   #define STR(x) #x
   char *s = STR(hello world);
   // CHECK: char * s = "hello world" ;
   ```
 
d. tests/lit/preprocess/paste.c:
   ```c
   #define CONCAT(a, b) a##b
   int CONCAT(my, var) = 1;
   // CHECK: int myvar = 1 ;
   ```
 
e. tests/lit/preprocess/conditionals.c:
   ```c
   #define DEBUG 1
   #if DEBUG
   int debug_mode = 1;
   #else
   int debug_mode = 0;
   #endif
   // CHECK: int debug_mode = 1 ;
   ```
 
f. tests/lit/preprocess/include.c:
   Create a fixture: tests/lit/preprocess/fixtures/values.h containing `#define VAL 99`
   Main file:
   ```c
   #include "fixtures/values.h"
   int x = VAL;
   // CHECK: int x = 99 ;
   ```
 
g. tests/lit/preprocess/complex.c:
   A file combining macros, conditionals, includes, stringify, paste:
   ```c
   #include "fixtures/values.h"
   #define MAKE_NAME(prefix, id) prefix##id
   #define SHOW(x) #x
   #if VAL > 50
   int MAKE_NAME(big_, value) = VAL;
   char *name = SHOW(MAKE_NAME(big_, value));
   #else
   int small_value = 0;
   #endif
   // CHECK: int big_value = 99 ;
   // CHECK: char * name = "MAKE_NAME(big_, value)" ;
   ```
 
h. tests/lit/preprocess/cli_define.c:
   Test -D flag. Run with: forge -E -D CUSTOM_VAL=777 cli_define.c
   ```c
   int x = CUSTOM_VAL;
   // CHECK: int x = 777 ;
   ```
   (The test harness needs to support passing CLI flags — if the current lit runner
   doesn't support this, add a mechanism like `// RUN: forge -E -D CUSTOM_VAL=777 %s`)
 
────────────────────────────────────────────────────────
SECTION 5 — Integration test
────────────────────────────────────────────────────────
 
Add one integration test (in tests/integration/) that runs the forge binary
as a subprocess:
 
```rust
#[test]
fn forge_e_flag_produces_output() {
    // Create a temp .c file with a simple macro
    // Run: forge -E <tempfile>
    // Assert: stdout contains expanded output
    // Assert: exit code is 0
}
```
 
This verifies the full CLI → driver → preprocessor → output pipeline works end-to-end.
 
────────────────────────────────────────────────────────
SECTION 6 — Verification
────────────────────────────────────────────────────────
 
Run:
- cargo test --all → all tests pass (existing 460 + new ones)
- cargo clippy --all-targets --all-features -- -D warnings → clean
- cargo fmt --all -- --check → clean
- Manual smoke test: `cargo run -- -E some_test_file.c` and inspect output visually
```

### Prompt 2.8 — Comprehensive validation (same pattern as lexer)

```
Run a FULL validation of the forge_preprocess crate and the driver integration before
we move to Phase 3 (Parser). This is the same pattern we used for the lexer — systematic
audit, completeness check, stress testing, and performance measurement.
 
Do the following parts IN ORDER. Do not skip any part.
 
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 1 — Code Audit
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 
A. unwrap()/expect() audit:
   - List EVERY unwrap() and expect() call in the forge_preprocess crate
   - For each one, decide: is it justified (e.g., guaranteed by invariant)?
     If yes, add a comment explaining the invariant.
     If no, replace with proper error handling (return Result, emit diagnostic, etc.)
   - The preprocessor should NEVER panic on any input. All errors become diagnostics.
 
B. TODO/FIXME audit:
   - Find every TODO, FIXME, HACK, XXX comment in forge_preprocess AND forge_driver
   - For each one: either resolve it now, or document it in KNOWN_ISSUES.md with
     a clear description of what's missing and when it should be addressed
 
C. Clippy strict:
   - Run: cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic
   - Fix everything except items you explicitly #[allow] with a justification comment
   - Then run normal clippy: cargo clippy --all-targets --all-features -- -D warnings
 
D. Dead code:
   - Check for pub functions/methods in forge_preprocess that are never called
     from outside the crate (except in tests)
   - Either make them pub(crate), remove them, or document why they're pub
 
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 2 — Completeness Matrix
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 
For EVERY feature below, verify a test exists AND passes.
If a test is missing, WRITE ONE. Fill in the table and include it in your final report.
 
MACRO DEFINITION & EXPANSION:
| Feature                              | Test? | Pass? |
|--------------------------------------|-------|-------|
| #define object-like                   |       |       |
| #define function-like                 |       |       |
| #define variadic (__VA_ARGS__)        |       |       |
| #undef                               |       |       |
| # (stringify)                         |       |       |
| Stringify preserves raw args          |       |       |
| Stringify escapes " and \             |       |       |
| ## (token paste)                      |       |       |
| Paste uses raw args                   |       |       |
| Paste producing invalid token → warn  |       |       |
| Self-referential macro (#define X X)  |       |       |
| Mutual recursion (A→B→A)             |       |       |
| Empty macro (#define EMPTY)           |       |       |
| Empty arguments F(,)                  |       |       |
| Func-like not invoked (no parens)     |       |       |
| Nested macro calls                    |       |       |
| Macro redefine same → no warning      |       |       |
| Macro redefine different → warning    |       |       |
 
CONDITIONAL COMPILATION:
| Feature                              | Test? | Pass? |
|--------------------------------------|-------|-------|
| #if with integer arithmetic           |       |       |
| #if with defined()                    |       |       |
| #if with defined X (no parens)        |       |       |
| #if unsigned arithmetic (-1 < 1U)     |       |       |
| #if ternary operator (? :)            |       |       |
| #if character literal ('A' == 65)     |       |       |
| #ifdef                                |       |       |
| #ifndef                               |       |       |
| #elif                                 |       |       |
| #elif chain (first true wins)         |       |       |
| #else                                 |       |       |
| #endif                                |       |       |
| Nested conditionals                   |       |       |
| #if 0 skipping arbitrary junk         |       |       |
| #error inside #if 0 → silent          |       |       |
| #warning inside #if 0 → silent        |       |       |
| Unmatched #endif → error              |       |       |
| Unmatched #if at EOF → error          |       |       |
| #elif after #else → error             |       |       |
| Lenient eval (unknown func → 0)       |       |       |
 
INCLUDE:
| Feature                              | Test? | Pass? |
|--------------------------------------|-------|-------|
| #include "file"                       |       |       |
| #include <file>                       |       |       |
| #include MACRO (computed)             |       |       |
| Nested includes (A includes B)        |       |       |
| Relative path (A includes "sub/B")    |       |       |
| Circular include → error              |       |       |
| Include depth limit (200) → error     |       |       |
| File not found → error                |       |       |
| #pragma once                          |       |       |
| Include guard detection               |       |       |
| Include guard + double include        |       |       |
 
DIRECTIVES:
| Feature                              | Test? | Pass? |
|--------------------------------------|-------|-------|
| #error (emits error diagnostic)       |       |       |
| #error with no message                |       |       |
| #warning (emits warning)              |       |       |
| #line NUMBER                          |       |       |
| #line NUMBER "FILENAME"               |       |       |
| #line with macro expansion            |       |       |
| #line inside include is local         |       |       |
| #pragma once                          |       |       |
| #pragma message                       |       |       |
| #pragma unknown → silent ignore       |       |       |
| _Pragma("once")                       |       |       |
| _Pragma via macro expansion           |       |       |
| _Pragma with non-string → error       |       |       |
| Null directive (bare #)               |       |       |
 
PREDEFINED MACROS:
| Feature                              | Test? | Pass? |
|--------------------------------------|-------|-------|
| __FILE__ (dynamic)                    |       |       |
| __LINE__ (dynamic)                    |       |       |
| __FILE__ changes in include           |       |       |
| __DATE__ (Mmm dd yyyy format)         |       |       |
| __TIME__ (hh:mm:ss format)            |       |       |
| __STDC__ == 1                         |       |       |
| __STDC_VERSION__ == 201710L           |       |       |
| __GNUC__ defined                      |       |       |
| Platform macros (__linux__/__APPLE__) |       |       |
| __SIZEOF_INT__ etc.                   |       |       |
 
BUILTINS:
| Feature                              | Test? | Pass? |
|--------------------------------------|-------|-------|
| __has_include(<existing>)  → 1        |       |       |
| __has_include("missing")   → 0        |       |       |
| __has_include in #if expression       |       |       |
| __has_builtin(x) → 0                  |       |       |
| __has_attribute(x) → 0                |       |       |
| __has_feature(x) → 0                  |       |       |
 
CLI & DRIVER:
| Feature                              | Test? | Pass? |
|--------------------------------------|-------|-------|
| -E flag produces output               |       |       |
| -I adds include path                  |       |       |
| -I order is respected                 |       |       |
| -D FOO (defines as 1)                 |       |       |
| -D FOO=value                          |       |       |
| -D function-like macro                |       |       |
| -U undefines macro                    |       |       |
| -D then -U removes it                 |       |       |
| tokens_to_source reconstruction       |       |       |
 
SYSTEM HEADERS:
| Feature                              | Test? | Pass? |
|--------------------------------------|-------|-------|
| #include <stddef.h>  → no errors      |       |       |
| #include <stdint.h>  → no errors      |       |       |
| #include <limits.h>  → no errors      |       |       |
| #include <stdio.h>   → no errors      |       |       |
| #include <stdlib.h>  → no errors      |       |       |
| #include <string.h>  → no errors      |       |       |
 
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 3 — Edge Case Stress Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 
Feed each of these through the preprocessor. Verify: NO panics, NO crashes.
Errors/warnings are expected for some — the point is graceful handling.
 
Write each as an actual test (not just manual verification).
 
1.  Empty file (zero bytes)
2.  File with only comments: `/* nothing here */ // also nothing`
3.  File with only whitespace and newlines
4.  #define with no body: `#define EMPTY` then use EMPTY
5.  #define with very long replacement (generate 1000 tokens programmatically)
6.  100 levels of nested #if: `#if 1\n` repeated 100 times, then `#endif\n` 100 times
7.  #include that doesn't exist → proper error diagnostic (not panic)
8.  Macro with 50 parameters (generate programmatically)
9.  Token paste producing invalid token (e.g., `##` of `+` and `*`) → should warn
10. Deeply nested include (A includes B includes C ... 50 levels, not circular) → should work
11. Very long #if expression: `#if 1+1+1+1+1+...` (100 terms) → should evaluate
12. #define producing another #define: `#define MAKE #define` then `MAKE FOO 1`
    → should NOT create a new macro (directives from expansion are not processed)
13. Stringification of very long argument (500 tokens)
14. Token paste chain: `a##b##c##d##e` (5 pastes in one replacement)
15. Macro expanding to itself with extra tokens: `#define A A A A` → terminates
16. UTF-8 in macro names (if any — should either work or give clean error)
17. #if with division by zero: `#if 1/0` → warning, treated as 0
18. Unterminated function-like macro invocation: `FOO(a, b` (no closing paren) → error
19. Very deeply nested parentheses in macro argument: `F(((((((((1)))))))))` → works
20. #include with absolute path: `#include "/dev/null"` → should work (empty file)
 
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 4 — System Header Smoke Test via forge -E
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 
This tests the FULL pipeline end-to-end (CLI → driver → preprocessor → output).
 
For each of the following, create a temporary .c file and run `forge -E` on it
as a subprocess. Assert: exit code 0, stderr has no errors.
 
1. `#include <stddef.h>`
2. `#include <stdint.h>`
3. `#include <limits.h>`
4. `#include <stdio.h>`
5. `#include <stdlib.h>`
6. `#include <string.h>`
7. `#include <errno.h>`
8. `#include <assert.h>`
9. `#include <ctype.h>`
10. `#include <math.h>`
 
If any fail, investigate and fix. Then re-run all tests.
 
Also test a combined file:
```c
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(void) {
    printf("hello\n");
    return 0;
}
```
Preprocess with `forge -E` and verify the output is valid (no error exit code).
 
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 5 — Performance Benchmark
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 
Create a benchmark test (can be a #[test] with timing, or a standalone binary):
 
Test A — Single heavy include:
   Create a file with just `#include <stdio.h>` and preprocess it.
   Measure wall-clock time. Run 5 times, report median.
   Target: < 500ms in debug mode, < 100ms in release mode.
 
Test B — Multi-include:
   ```c
   #include <stdio.h>
   #include <stdlib.h>
   #include <string.h>
   #include <stdint.h>
   #include <limits.h>
   #include <errno.h>
   #include <ctype.h>
   #include <math.h>
   #include <assert.h>
   #include <stddef.h>
   ```
   Preprocess and measure. Include guard / pragma once detection should prevent
   redundant processing of shared transitive includes.
   Target: should NOT be 10x slower than Test A (due to dedup).
 
Test C — Token count:
   After preprocessing Test B, report the total number of output tokens.
   (Just informational — good to know the scale.)
 
If any benchmark misses the target, profile with:
   `cargo build --release && time cargo run --release -- -E test.c > /dev/null`
   Report which functions are hot (if you can identify them).
 
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 6 — Final Report
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
 
Produce a summary report with:
 
1. Code audit results:
   - Number of unwrap/expect found → how many replaced → how many justified
   - Number of TODO/FIXME found → how many resolved → how many documented
   - Clippy pedantic results
2. Completeness matrix (the filled-in table from Part 2)
   - Any missing tests that were added
   - Any features that don't pass → what's the issue?
3. Edge case results:
   - Which of the 20 stress tests passed/failed
   - Any panics found → fixed?
   - Any new bugs discovered → fixed?
4. System header results:
   - Which headers pass/fail via forge -E
   - Any fixes needed
5. Performance numbers:
   - Test A median time (debug + release)
   - Test B median time (debug + release)
   - Token counts
   - Any performance concerns
6. Verdict: is forge_preprocess ready for Phase 3?
Run one final time:
   cargo test --all
   cargo clippy --all-targets --all-features -- -D warnings
   cargo fmt --all -- --check
 
All must be green. Report the total test count.
```

---

## Pitfalls & Debugging Tips

### "Stringify uses raw args, normal substitution uses expanded args"
This is the #1 source of preprocessor bugs. Example:
```c
#define A 42
#define STR(x) #x
#define VAL(x) x
STR(A)  → "A"   (raw — A is NOT expanded before stringify)
VAL(A)  → 42    (expanded — A IS expanded before substitution)
```
If your stringify produces `"42"` instead of `"A"`, the argument expansion is happening too early.

### "Stringify must escape quotes and backslashes"
```c
#define STR(x) #x
STR("hello\n")  → "\"hello\\n\""
```
If your stringify produces `""hello\n""` (without escaping), any downstream parser will choke on the broken string literal. Walk the argument tokens: when you encounter a StringLiteral or CharLiteral, iterate its raw characters and prefix every `"` with `\` and every `\` with `\`.

### "#if unsigned arithmetic"
```c
#if -1 < 1U
// This is FALSE in C! -1 promotes to unsigned → UINTMAX_MAX → not less than 1
#endif
```
If your evaluator uses i64 for everything, `-1 < 1` evaluates to true (wrong). Track signedness of each value using a `PPValue { Signed(i64) | Unsigned(u64) }` and apply C's usual arithmetic conversions: when mixing signed and unsigned, convert both to unsigned.

### "Token paste uses raw args too"
```c
#define A hello
#define PASTE(x, y) x##y
PASTE(A, _world)  → A_world   (NOT hello_world)
```
If you get `hello_world`, you're expanding args before pasting.

### "#if 0 blocks can contain anything"
```c
#if 0
This is not valid C: @#$%^& "unterminated string
But the preprocessor MUST handle it — just skip to #endif.
#endif
```
When skipping an inactive block, do NOT try to lex or validate the content. Just scan for `#` at start of line and check for `if`/`ifdef`/`ifndef`/`elif`/`else`/`endif`.

**WAIT — this is a real problem.** If the lexer has already tokenized the entire file (including the `#if 0` block), and the lexer choked on invalid content inside the block, we have an issue. 

Solutions:
- Option A: Have the lexer be lenient enough that it doesn't choke (produces Unknown tokens for garbage) — this is already the case from Phase 1.
- Option B: Do lazy lexing (lex on demand, not the whole file upfront) — more complex.

Since the Phase 1 lexer already produces Unknown tokens for invalid characters and recovers from unterminated strings, Option A should work. But TEST THIS specifically.

### "has_leading_space matters for function-like detection"
```c
#define FOO (x)  // Object-like: expands to "(x)" — space before (
#define BAR(x)   // Function-like: takes parameter x — no space before (
```
The ONLY difference is `has_leading_space` on the LeftParen token. If this flag is wrong, function-like macros will be misclassified as object-like and vice versa. Triple-check this with a test.

### "System headers are the boss fight"
`#include <stdio.h>` on Ubuntu will pull in 20+ files and use every preprocessor feature. The most common blockers:
1. Missing `__GNUC__` macro — define it as `4` 
2. Missing `__has_builtin` / `__has_attribute` / `__has_include` — implement as macros that return 0 (or implement `__has_include` properly since we CAN check if a file exists)
3. `__builtin_va_list` — this appears as a type in `<stdarg.h>`. The PREPROCESSOR doesn't need to understand it — just pass the tokens through. The parser will need to handle it.
4. `__extension__` — GCC keyword meaning "suppress warnings." Just pass it through as a token.
5. `__restrict` / `__inline` / `__volatile__` — GCC aliases for restrict/inline/volatile. Pass through.

The preprocessor's job is just to expand macros and process directives. If the resulting token stream has GNU-specific tokens, that's fine — the parser deals with them.

---

## Notes

- **Test after every prompt.** Run `cargo test --all` after each prompt before proceeding to the next. Don't accumulate untested code.
- **The preprocessor should never panic.** Invalid input produces diagnostics, not crashes.
- **Performance matters but correctness matters more.** If the preprocessor is slow but correct, we can optimize later. If it's fast but wrong, we have to debug subtle miscompilations for weeks.
- **Keep the diagnostic quality high.** "#include <nosuchfile.h>" should say: `error: 'nosuchfile.h' file not found` with the span of the #include directive, not a generic "file not found" with no context.