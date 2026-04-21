# Phase 2 Multi-file Span Fix — Final Report

Report covering all four sub-prompts (2F.1 → 2F.2 → 2F.3 → 2F.4).

## Summary of what changed

The pipeline went from "every span is a `(start, end)` pair measured in
bytes of the primary source" to "every span names a `FileId` (multi-file
support) and an `ExpansionId` (macro-expansion backtrace)."  The work
spanned four sub-prompts:

| Sub-prompt | Focus |
|------------|-------|
| 2F.1 | `SourceMap` / `SourceFile` / `FileId` + `line_starts` table |
| 2F.2 | `Span.file: FileId`; render through `SourceMap`; every lexer/preprocessor token lexed in a given file carries that file's id |
| 2F.3 | `ExpansionFrame` / `ExpansionTable` + `Span.expanded_from`; preprocessor stamps every expansion-produced token; renderer walks the backtrace and emits `in expansion of macro 'X'` auxiliary labels |
| 2F.4 (this doc) | Regression guards, edge-case tests, documentation, `Span::primary` audit |

Concretely, `Span` now is:

```rust
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
    pub expanded_from: ExpansionId,
}
```

and the `CompileOutput` returned from `forge_driver::compile` carries
both a `SourceMap` and an `ExpansionTable`.  Every error, warning, or
note anchors on a `Span` that transparently resolves to the right file
and the right macro-invocation chain when rendered.

## Test count delta

| Milestone | Pass / Fail / Ignored |
|-----------|------------------------|
| Before Phase 2 fix (end of Phase 4) | ≈1300 / 0 / 1 |
| After 2F.3 | 1340 / 0 / 1 |
| After 2F.4 (this sub-prompt) | **1346 / 0 / 1** |

The +6 new tests added in 2F.4:

- `source_file_no_trailing_newline_last_line_reachable` (forge_diagnostics)
- `source_file_utf8_multibyte_column_is_byte_offset_not_grapheme` (forge_diagnostics)
- `source_file_bom_at_start_is_retained_in_column_indexing` (forge_diagnostics)
- `source_file_very_long_line_does_not_overflow_u32` (forge_diagnostics)
- `regression_user_error_reports_user_file` (forge_driver)
- `regression_macro_error_emits_backtrace` (forge_driver)

## Performance

Full-pipeline Test A (`int main(void) { return 0; }`), release, 3 runs:

| Stage | Run 1 | Run 2 | Run 3 |
|-------|-------|-------|-------|
| Before Phase 2 fix | 9 ms | 7 ms | 7 ms |
| After 2F.3 | 7 ms | 9 ms | 8 ms |
| After 2F.4 | 7 ms | 9 ms | 7 ms |

Budget: 80 ms release.  Phase 2F.3 acceptance target was "<2× baseline,
ideally <10 ms release" — comfortably met.  No observable regression
from any sub-prompt.

## ariadne multi-file rendering outcome

ariadne carried the multi-file rendering cleanly.  The only adaptation
needed was a small `Cache` shim (`SourceMapCache` inside
`forge_diagnostics`) that delegates source lookups to `SourceMap` by
`FileId`.  No workarounds, no patches to ariadne itself.

Macro backtraces render as stacked cyan auxiliary labels below the
primary error label, in innermost-first order.  The primary label
still points at the token that actually tripped the check, so the user
sees both "what is wrong" and "where the macro came from" in one
report.

## Deprecated shims

None to remove.  The pre-2F.2 render API was replaced in place rather
than kept as a deprecated parallel surface, so nothing was carrying a
`#[deprecated]` attribute to clean up in 2F.4.  (Confirmed via
`grep -rn "#[deprecated]" crates/` — no matches.)

## Span::primary audit

`grep -rn "Span::primary" crates/ | grep -v tests/ | grep -v "src/tests/"`
returned two production hits at the start of 2F.4:

| File | Before | After |
|------|--------|-------|
| `forge_parser/src/parser.rs:55` | `Span::primary(0, 0)` fallback when the input token vector is empty | `Span::new(FileId::INVALID, 0, 0)` with a doc comment — this is a true sentinel for "synthetic EOF with no source origin", and `FileId::INVALID` documents that intent cleanly |
| `forge_sema/src/stmt.rs:697` | `Span::primary(0, 0)` fallback when `param_decls.get(idx)` returned `None` | threaded `func.span` through `declare_parameters` as a `fallback_span`; the file id is now always a real one |

All other `Span::primary` occurrences are inside `tests/` or
`src/tests/` — legitimate single-file test usage, not touched.

`Span::primary`'s doc comment was rewritten to warn production callers
away:

> Intended for tests and sentinel spans. Production code should use
> `Span::new` with a real `FileId` from the `SourceMap` — using
> `Span::primary` inside the compiler will cause multi-file diagnostics
> to point at the wrong file whenever the `SourceMap` holds more than
> one entry.

## `SourceFile` edge-case documentation

Explicit tests now cover: empty file, no trailing newline, CRLF, UTF-8
multi-byte characters (column is byte-index, not grapheme-index), a
leading UTF-8 BOM (retained verbatim for v1 — documented choice), and
very long single lines (no `u32` overflow under the existing cap).
`SourceFile::line_col`'s doc comment was expanded to spell these
decisions out.

## Notable bug caught during 2F.4

While writing the macro-backtrace regression test, an ExpansionId leak
was discovered in `Parser::span_from`: it built wider spans via
`Span::new(...)`, which resets `expanded_from` to `NONE`.  AST-node
spans therefore lost the expansion id their starting token carried, and
diagnostics anchored on those wider spans could not render a
backtrace.  Fixed by chaining `.with_expansion(start.expanded_from)`
onto the rebuilt span, preserving the starting token's expansion id.

The existing `stress_50_nested_scopes` and `realworld.c` tests still
pass, so the fix does not perturb the common single-file path.

## Known follow-ups deferred to later phases

- **Macro-expansion spans in assembler/linker error paths** — once
  `forge_codegen` and the linker driver come online, their diagnostics
  should also anchor on `Span` so the backtrace infrastructure carries
  through to post-sema errors.  Current codegen is not yet implemented,
  so this is a Phase 7 concern.
- **`#line` directive interaction with `expanded_from`** — `#line N
  "filename"` currently reshapes `__LINE__` / `__FILE__` but does not
  yet invalidate or reconcile any pre-existing expansion ids on
  already-produced tokens.  In practice this is fine (tokens do not
  travel backwards through `#line`), but the interaction is worth
  re-audit when Phase 5 or 7 starts leaning on `#line` for generated
  code.
- **Pragma-driven source virtualisation** — no compiler we target
  emits or consumes pragmas that change `FileId` mid-stream, but if
  this becomes needed (e.g. `#pragma GCC poison` or GCC's line-marker
  comments from `-E -P`), the `SourceMap` interface is ready to absorb
  it without a core-data-model rework.
- **BOM stripping** — v1 retains the BOM.  If a future language-mode
  switch needs to treat the BOM as absent (e.g. for strict spec
  conformance of column numbers on line 1), it is a localised change
  inside `SourceFile::new` and its callers.

## Verdict

**FIX COMPLETE.**

All four sub-prompts landed, all 4 gates green
(`build`/`test`/`clippy`/`fmt`), full-pipeline perf inside the Phase 4
budget, multi-file and macro-backtrace rendering exercised end-to-end
with dedicated regression tests.  No outstanding correctness gaps at
the current pipeline scope (lex → preprocess → parse → sema).
