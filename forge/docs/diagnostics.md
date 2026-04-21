# Diagnostics pipeline

Forge produces user-facing errors, warnings, and notes through one
well-defined pipeline.  This document describes the shape of that
pipeline so a new compiler phase can wire into it correctly the first
time.

## Data model

Every diagnostic anchors on a `Span`.  A `Span` is a triple:

```rust
pub struct Span {
    pub file: FileId,          // which source file
    pub start: u32,            // inclusive byte offset
    pub end: u32,              // exclusive byte offset
    pub expanded_from: ExpansionId, // macro-expansion origin, or NONE
}
```

Two registries disambiguate the two indirections:

- **`SourceMap`** — owns every source file read by the compiler.  The
  translation-unit root is `FileId::PRIMARY`; each `#include`d header
  gets a fresh `FileId` allocated sequentially.  Every `Span` carries
  the `FileId` of the file the token was *lexed from* — a `42` produced
  by expanding `#define N 42` keeps the header's `FileId` even when it
  lands in `main.c`'s token stream.
- **`ExpansionTable`** — owns every macro expansion the preprocessor
  performed.  Each entry is an `ExpansionFrame` carrying the macro
  name, the invocation span (where the user wrote `BAD_INIT`), the
  definition span (where `#define BAD_INIT …` lives), and a parent
  link to the enclosing expansion (or `ExpansionId::NONE` for a
  top-level invocation).  Tokens produced by an expansion have their
  `span.expanded_from` stamped with the frame's id.

## Pipeline

```text
   source files  ─┐
                  ▼
             ┌────────┐
             │ lexer  │  every token's span.file = the FileId of the file
             └────────┘  it was lexed from (SourceMap allocates it)
                  │
                  ▼
           ┌──────────────┐
           │ preprocessor │  every expansion-produced token's
           │              │   span.expanded_from = a fresh ExpansionId
           │              │   from ExpansionTable
           └──────────────┘
                  │
                  ▼
             ┌────────┐
             │ parser │  AST-node spans are built from token spans;
             └────────┘  span_from() preserves start.expanded_from
                  │      so wider spans keep the macro backtrace
                  ▼
             ┌────────┐
             │  sema  │  Diagnostics anchor on AST / token spans;
             └────────┘  spans are already correct, nothing extra
                  │      to thread through
                  ▼
         ┌─────────────────┐
         │ render_diagnostics
         │ (forge_diagnostics)│  Walks SourceMap for file lookup +
         │                 │   ExpansionTable for macro backtrace
         └─────────────────┘
                  │
                  ▼
             ariadne report
```

## Adding a new diagnostic

In the common case you do *nothing special*: grab the span from the
AST or token, pass it to `Diagnostic::error(...).span(...)`, and the
renderer does the rest.

```rust
use forge_diagnostics::Diagnostic;

ctx.emit(
    Diagnostic::error("expected integer initializer")
        .span(expr.span)                   // from the AST
        .label("this is a string literal") // optional secondary
        .note("integer initializers must be integer constants"),
);
```

The renderer:
- looks up `expr.span.file` in the `SourceMap` → gets the file name and
  source text,
- walks `expr.span.expanded_from` through `ExpansionTable::backtrace`
  → gets zero or more `in expansion of macro 'X'` auxiliary labels,
- hands everything to `ariadne` for a single pretty report.

No per-phase boilerplate, no manual include-stack tracking, and the
same diagnostic produces a readable rendering whether the offending
token came from the primary source file, a `#include`d header, or a
macro expansion three levels deep.

## Do / don't

- **Do** carry spans directly from tokens through AST into sema and
  beyond.  They already carry everything needed.
- **Do** use `Span::new(file, start, end)` when you need to *extend*
  a span to cover a wider range; call `.with_expansion(id)` (or go
  through `Parser::span_from`) to preserve the starting span's
  expansion id.
- **Don't** call `Span::primary(...)` inside the compiler — it hard-
  codes `FileId::PRIMARY` and will mis-attribute diagnostics once the
  `SourceMap` holds more than one file.  `Span::primary` is for tests
  and single-file fixtures.
- **Don't** fabricate `FileId::INVALID` spans for real source locations
  — that sentinel is only for compiler-synthesised origins (e.g. the
  EOF after an empty token vector, or the `define_span` placeholder
  for built-in magic macros like `__LINE__`).
