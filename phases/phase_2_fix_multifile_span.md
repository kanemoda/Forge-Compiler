# Phase 2 Fix — Multi-file Spans and Macro Expansion Tracking

**Why this exists:** Span is currently `{ start: u32, end: u32 }` — no file identity, no macro origin. Every diagnostic from `#include <stdio.h>`-expanded code reports only a byte offset, losing file and macro context. Phase 4 sema passes because the smoke tests produce user-code diagnostics. Real debugging is painful today. Phase 5+ will be worse.

**Scope:** Extend `forge_lexer`, `forge_preprocess`, `forge_parser`, `forge_diagnostics` to track source-file identity AND macro expansion chains through the entire token pipeline. Diagnostics render across multiple files with "in expansion of macro X" backtraces.

**Preconditions:**
- Phase 4 is complete and committed on main.
- `cargo test --all` is green (~1325 tests).
- `cargo clippy --all-targets --all-features -- -D warnings` is clean.

**Estimated effort:** 2-3 days across 4 sub-prompts.

**Scale of test migration:** The project has ~1325 tests, many constructing `Span` manually (`Span::new(0, 5)`). Most will compile-error after 2F.2 until migrated. Budget real time for this — it's mechanical but not instantaneous.

---

## Workflow (applies to every sub-prompt)

1. Feed ONE sub-prompt at a time to Claude Code. Do not paste all four at once.
2. At the end of each sub-prompt, Claude Code runs the gate commands with **explicit exit codes** and STOPS. It does NOT commit.
3. You review the report, run any sanity checks, and commit manually when satisfied.
4. Only then feed the next sub-prompt.

Gate commands for every sub-prompt:

```
cargo build --all                                        → exit 0
cargo test --all                                         → all pass
cargo clippy --all-targets --all-features -- -D warnings → exit 0
cargo fmt --all -- --check                               → exit 0
```

Every gate must show explicit exit 0 or "0 failed". A gate that says "clean" without an exit code is NOT acceptable — that pattern caused a regression in Phase 4.3.1 and was explicitly called out there.

---

## Sub-prompt 2F.1 — FileId and SourceMap infrastructure

```
This is sub-prompt 1 of 4 for the Phase 2 multi-file Span fix.

Goal: introduce FileId, SourceFile, and SourceMap types in forge_diagnostics.
This sub-prompt adds infrastructure ONLY. No existing code changes behavior.
All ~1325 existing tests must still pass unchanged.

Same workflow as Phase 4: explicit exit codes at the end, do NOT commit —
report and I will commit manually.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — FileId type
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create crates/forge_diagnostics/src/source_map.rs (or similarly named
module — check existing conventions in lib.rs).

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct FileId(pub u32);

impl FileId {
    pub const INVALID: FileId = FileId(u32::MAX);
    pub const PRIMARY: FileId = FileId(0);   // the translation unit root
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — SourceFile
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct SourceFile {
    pub id: FileId,
    pub name: String,             // display path ("main.c", "/usr/include/stdio.h")
    pub source: String,           // full file contents
    pub line_starts: Vec<u32>,    // byte offset of each line's start
                                   // line_starts[0] == 0 always
                                   // line_starts.len() == number of lines
}

impl SourceFile {
    pub fn new(id: FileId, name: String, source: String) -> Self {
        // Build line_starts eagerly:
        //   starts with 0
        //   append i+1 for each \n found (byte AFTER the newline)
        //   "\r\n" — \n is what starts the new line
        //   Empty file: line_starts == vec![0]; line_col(0) → (1, 1)
    }

    /// Byte offset → (1-based line, 1-based column).
    /// Offsets past EOF saturate to the last valid position — do not panic.
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        // Binary search on line_starts.
        // Column = offset - line_starts[line_index] + 1 (1-based, byte-indexed).
    }
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — SourceMap
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self;

    /// Adds a new file. Returns a fresh FileId sequentially (0, 1, 2, ...).
    pub fn add_file(&mut self, name: String, source: String) -> FileId;

    pub fn get(&self, id: FileId) -> Option<&SourceFile>;

    /// Convenience for callers that know the FileId is valid. Panics on INVALID.
    /// Prefer get() in paths that might receive INVALID.
    pub fn get_or_panic(&self, id: FileId) -> &SourceFile;

    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn iter(&self) -> impl Iterator<Item = &SourceFile>;
}

impl Default for SourceMap { ... }

Export FileId, SourceFile, SourceMap from forge_diagnostics::lib.rs
alongside the existing Diagnostic exports.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create crates/forge_diagnostics/src/tests/source_map.rs (follow the
project's test organization rule; check forge_sema/src/tests/ for the
pattern). Required tests — each its own #[test] fn:

  fileid_invalid_constant_has_max_value
  fileid_primary_constant_is_zero
  add_file_returns_sequential_ids
  source_file_line_col_offset_zero_is_line_one_col_one
      (empty string source + offset 0 → (1, 1))
  source_file_line_col_first_line_middle
      "hello\nworld" + offset 3 → (1, 4)
  source_file_line_col_second_line_start
      "hello\nworld" + offset 6 → (2, 1)
  source_file_line_col_crlf_handled
      "foo\r\nbar" + offset 5 → (2, 1)
  source_file_line_col_past_eof_saturates
      (offset past end must not panic; returns last valid position)
  source_map_get_invalid_returns_none
  source_map_len_grows_with_add_file
  source_map_iter_yields_in_insertion_order

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Discipline
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Do NOT modify Span yet.
Do NOT modify the preprocessor yet.
Do NOT modify render_diagnostics yet.
Do NOT replace any existing HashMap with FxHashMap.

The ONLY goal is to land the plumbing as a dead-code addition. The
existing ~1325 tests continue passing because nothing references FileId
/ SourceFile / SourceMap yet.

If you find yourself wanting to touch Span or render_diagnostics, stop —
that's 2F.2, not this.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
CHECKPOINT
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

cargo build --all                                        → exit 0
cargo test --all                                         → all pass;
                                                            total count
                                                            unchanged except
                                                            +~10 source_map tests
cargo clippy --all-targets --all-features -- -D warnings → exit 0
cargo fmt --all -- --check                               → exit 0

STOP. Do not commit. Report:
  - All four gate exit codes
  - Test count delta (expected: +10 or so)
  - Any unexpected findings
```

---

## Sub-prompt 2F.2 — Span carries FileId

```
This is sub-prompt 2 of 4. 2F.1 landed FileId/SourceFile/SourceMap as
dead-code infrastructure.

Goal: extend Span to carry FileId. Big mechanical step — four crates
change, hundreds of test call sites need migration.

IMPORTANT: 2F.2 either lands cleanly (all four gates green) or doesn't
land at all. A half-done Span refactor leaves the project uncompilable.
If you get stuck partway, REPORT where and what blocked you rather than
forcing a partial state.

Same workflow: explicit exit codes. Every gate must show exit 0 or
"0 failed". The "reports 'clean' without verification" anti-pattern is
forbidden — it caused a regression in Phase 4.3.1 and was called out.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Span format
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

In forge_lexer/src/lib.rs (current home of Span):

pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

Add forge_diagnostics as a dependency of forge_lexer (one-line
Cargo.toml change). Import FileId.

Constructors and helpers:

    /// Primary constructor. Production code passes a FileId from SourceMap.
    pub const fn new(file: FileId, start: u32, end: u32) -> Self;

    /// Tests and sentinel/single-file scenarios. Sets file = FileId::PRIMARY.
    /// Production code should prefer Span::new with a real FileId.
    pub const fn primary(start: u32, end: u32) -> Self;

    pub const fn len(&self) -> u32;
    pub const fn is_empty(&self) -> bool;
    pub fn range(&self) -> std::ops::Range<usize>;

Update Display to render "file_id:start..end" so debug output
distinguishes files.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Lexer propagates FileId
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    impl Lexer {
        pub fn new(source: &str, file_id: FileId) -> Self;
    }

Every Span the lexer produces stamps file = file_id.

lex_fragment similarly:
    pub fn lex_fragment(source: &str, file_id: FileId) -> ...

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Preprocessor tracks files via SourceMap
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Add to Preprocessor:

    source_map: SourceMap,

Construction with root source:
    let root_id = source_map.add_file(root_name, root_source);
Every PPToken from the initial lexer carries file = root_id.

On #include of a new file:
    let contents = /* read file from disk */;
    let new_id = self.source_map.add_file(header_path, contents);
    let lexer = Lexer::new(&contents, new_id);
    // push include frame

Every PPToken from that lexer carries file = new_id.

Expose SourceMap after preprocessing. Pick the pattern that fits
existing API — either:
    pub fn finish(self) -> (Vec<PPToken>, SourceMap)
OR:
    pub fn source_map(&self) -> &SourceMap
    pub fn into_source_map(self) -> SourceMap

The parser and diagnostic renderer need SourceMap to reach them.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Parser propagates
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

The parser does not modify spans — it just propagates them into AST
nodes. The parser's public API grows to thread SourceMap through (or
accept/return it). Adjust based on the existing signature; the goal is
that SourceMap reaches the diagnostic renderer.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Diagnostics render with SourceMap
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Current render_diagnostics takes (source: &str, filename: &str).
Replace:

    pub fn render_diagnostics(
        source_map: &SourceMap,
        diagnostics: &[Diagnostic],
    );

    pub fn render_diagnostics_to_string(
        source_map: &SourceMap,
        diagnostics: &[Diagnostic],
    ) -> String;

For each diagnostic's primary label and secondary labels:
  - Look up span.file in SourceMap
  - Use that SourceFile's name and source in ariadne's Report

Ariadne supports multi-file rendering. Review ariadne's Cache trait —
wrapping SourceMap in a newtype that implements ariadne::Cache<FileId>
is the idiomatic path.

TECHNICAL RISK — ariadne multi-file API:
If ariadne's multi-file rendering proves awkward for this layering,
stop and report. Do not silently fall back to rendering only one
file. The whole point is multi-file fidelity.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 6 — Test migration (the mechanical part)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Hundreds of tests construct Spans manually. Patterns:

OLD:
    let span = Span::new(0, 5);
NEW (test has single implicit file):
    let span = Span::primary(0, 5);

OLD (token builders):
    Token { kind, span: Span { start: 0, end: 5 }, ... }
NEW:
    Token { kind, span: Span::primary(0, 5), ... }

Shared helpers. In each affected crate's tests/helpers.rs (create if
absent):

    pub fn single_file_map(src: &str) -> (FileId, SourceMap) {
        let mut sm = SourceMap::new();
        let id = sm.add_file("<test>".into(), src.into());
        (id, sm)
    }

    pub fn test_span(start: u32, end: u32) -> Span {
        Span::primary(start, end)
    }

Volume strategy:
  1. Change Span struct first. Every caller breaks.
  2. Fix lexer and preprocessor construction sites.
  3. Compile; let the compiler list every test needing migration.
  4. For each error: Span::primary (single-file tests) or build a
     SourceMap and use Span::new.
  5. Do NOT delete tests to silence errors. If a test is broken by
     the migration, flag it in the report with the reason.

sed-based bulk migration is OK for trivial rewrites:
    Span::new(\([0-9]*\), \([0-9]*\)) → Span::primary(\1, \2)

But review each hunk — some Span::new in production code has a real
FileId available and should NOT be rewritten to primary.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 7 — Sanity integration test
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Add crates/forge_preprocess/tests/multifile_diag.rs (or similar
integration test location).

Write a main file that #includes "helper.h" where helper.h defines
MY_MACRO. Run the preprocessor; inspect tokens:

  - Tokens from the main file have span.file == FileId of main.c
  - Tokens from MY_MACRO's expansion body have span.file == FileId of
    helper.h (because the macro body was lexed from helper.h)

We are NOT yet testing macro backtrace (that's 2F.3). This test only
confirms multi-file Span propagation works end-to-end.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
CHECKPOINT
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

cargo build --all                                        → exit 0
cargo test --all                                         → all pass
cargo clippy --all-targets --all-features -- -D warnings → exit 0
cargo fmt --all -- --check                               → exit 0

STOP. Do not commit. Report:
  - All four gate exit codes
  - Test count before and after (should be unchanged modulo the new
    multifile_diag integration test)
  - Rough count of test call sites migrated
  - Whether ariadne multi-file rendering worked
  - Any tests blocked during migration with explanations
```

---

## Sub-prompt 2F.3 — Macro expansion tree

```
This is sub-prompt 3 of 4. 2F.1 built infrastructure; 2F.2 threaded
FileId through every Span.

Goal: track the macro-expansion origin of every token. After 2F.3, a
diagnostic inside a macro expansion reports the invocation chain
("expanded from macro FOO, invoked at line N of main.c, ...").

Same workflow: explicit exit codes, do NOT commit.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Dependency direction decision
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

ExpansionId needs to live in a crate that forge_lexer can import
(because Span must carry it). forge_preprocess already depends on
forge_lexer, so we CANNOT put ExpansionId in forge_preprocess.

Options:
  (a) Put ExpansionId in forge_diagnostics as a bare u32 wrapper. The
      ExpansionTable (the actual data structure) stays in forge_preprocess.
      Both sides see ExpansionId.
  (b) Create a new forge_span crate that just holds Span, FileId, and
      ExpansionId. Both lexer and preprocess depend on it.

Pick (a) unless there's a strong reason to add another crate. Document
the choice.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — ExpansionId, ExpansionFrame, ExpansionTable
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

In forge_diagnostics (or wherever chosen in SECTION 1):

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct ExpansionId(pub u32);

impl ExpansionId {
    pub const NONE: ExpansionId = ExpansionId(u32::MAX);
}

In forge_preprocess:

pub struct ExpansionFrame {
    pub id: ExpansionId,
    /// The span at which the macro name appeared pre-expansion.
    pub invocation_span: Span,
    /// The macro's name for diagnostic rendering.
    pub macro_name: String,
    /// The span of the #define line, for "defined here" notes.
    pub definition_span: Span,
    /// Parent expansion if nested; NONE for top-level.
    pub parent: ExpansionId,
}

pub struct ExpansionTable {
    frames: Vec<ExpansionFrame>,
}

impl ExpansionTable {
    pub fn new() -> Self;
    pub fn push(&mut self, frame: ExpansionFrame) -> ExpansionId;
    pub fn get(&self, id: ExpansionId) -> Option<&ExpansionFrame>;

    /// Walks the chain from `id` to the root. Returns innermost-first.
    pub fn backtrace(&self, id: ExpansionId) -> Vec<&ExpansionFrame>;
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Span gains expanded_from
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Extend Span:

pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
    pub expanded_from: ExpansionId,   // NONE for non-expanded tokens
}

Constructors:
    pub const fn new(file: FileId, start: u32, end: u32) -> Self
        (expanded_from = NONE)
    pub const fn primary(start: u32, end: u32) -> Self
        (file = PRIMARY, expanded_from = NONE)

Builder:
    pub fn with_expansion(mut self, id: ExpansionId) -> Self

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Preprocessor stamps expansions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

When expanding a macro M invoked at span S:

  1. id = expansion_table.push(ExpansionFrame {
         invocation_span: S,
         macro_name: M.name.clone(),
         definition_span: M.define_span,
         parent: if S.expanded_from == NONE { NONE }
                 else              { S.expanded_from },
     });
  2. For every token produced by expansion BODY, stamp
     token.span.expanded_from = id.

Nested expansions:
  - Macro A calls macro B. A's outer invocation → frame_A
    with parent=NONE.
  - B is invoked from a token in A's body whose expanded_from = A.
  - frame_B has parent = A.

Argument substitution (C17 §6.10.3.1):
Argument tokens retain their ORIGINAL expanded_from. Body tokens get
the NEW expansion's id.

Example:
  #define ID(x) x
  #define F(x) (x * 2)
  int z = F(ID(3));

  - `3` at user code: expanded_from = NONE initially.
  - ID's expansion stamps the body — which is just `x` substituting
    to `3`. After ID expansion, `3` carries ID's expansion id.
  - When F is expanded, the `x` substitutes to the arg tokens.
    The `3` keeps its ID id (arg preservation). The `(`, `*`, `2`,
    `)` from F's body get F's expansion id.

Builtin expansions (__LINE__, __FILE__, __DATE__, __TIME__):
The expanded tokens point (file/offset) to the USE SITE — this is
correct for diagnostics. Still stamp expanded_from so we know they
came from a builtin special form.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Expose ExpansionTable
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

The preprocessor's output now includes ExpansionTable. Update the
result struct (same pattern as SourceMap in 2F.2):

pub struct PreprocessResult {
    pub tokens: Vec<PPToken>,
    pub source_map: SourceMap,
    pub expansions: ExpansionTable,
}

The parser doesn't modify ExpansionTable — it forwards it through
to the renderer.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 6 — Diagnostics render expansion backtraces
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

render_diagnostics now takes (source_map, expansions, diagnostics).

For each diagnostic whose primary span has expanded_from != NONE:
  1. Emit the main error at span.file:span.start.
  2. Emit a secondary label: "in expansion of macro 'M'" pointing at
     the frame's invocation_span.
  3. If frame has a parent, recurse and emit another label for the
     parent's invocation.
  4. Stop at a frame with parent = NONE.

Example output target:

    error: incompatible types in assignment
      ┌─ main.c:3:12
    3 │     x = BAD(42);
      │         ^^^^^^^
    note: in expansion of macro 'BAD'
      ┌─ defs.h:7:17
    7 │ #define BAD(x) ((const char *)x)
      │                 ^^^^^^^^^^^^^^^^

Start simple: one note per frame showing the macro name and invocation
site. The definition_span can be a second row or deferred if ariadne
makes it awkward.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 7 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Upgrade the multifile_diag integration test from 2F.2:

macro_expansion_single_level:
    #define PI 314
    int x = PI;
  After preprocess: `314` has expanded_from != NONE, frame has
  macro_name == "PI", invocation_span points at `PI` in main.c.

macro_expansion_nested_two_levels:
    #define A(x) B(x)
    #define B(x) (x + 1)
    int y = A(42);
  - `+` token's expanded_from chain, via backtrace(), → [B, A].
  - `42` token's expanded_from == NONE.

macro_argument_preservation:
    #define ID(x) x
    #define F(x) (x * 2)
    int z = F(ID(3));
  - `3` preserves ID's expansion id.
  - `*` and `2` have F's expansion id.
  - backtrace on `*` → [F].

builtin_line_expansion_tracked:
    __LINE__ alone.
  - expanded_from != NONE.
  - Frame has macro_name == "__LINE__" (or similar).

diagnostic_emits_macro_backtrace:
  Feed a diagnostic whose primary span's expanded_from is set. Capture
  rendered output. Assert it contains both the main error and an "in
  expansion of" note with the macro name.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 8 — Performance sanity
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Phase 3 parser: 14ms stdio.h. Phase 4 Test A: 6ms full pipeline
(release). Re-measure after 2F.3.

Target: full pipeline Test A stays under 10ms release. Span gained
a u32 (4 bytes); ExpansionTable is ~hundreds to low-thousands of
frames per nontrivial TU. Memory bounded; CPU increase should be
sub-20%.

If substantially over (> 2x), profile and note the hot path in the
report, but don't optimize unless egregious.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
CHECKPOINT
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

cargo build --all                                        → exit 0
cargo test --all                                         → all pass
cargo clippy --all-targets --all-features -- -D warnings → exit 0
cargo fmt --all -- --check                               → exit 0

STOP. Do not commit. Report:
  - All four gate exit codes
  - Test count delta
  - Performance numbers (Test A before and after)
  - Which crate houses ExpansionId (SECTION 1 decision)
  - Any edge cases found during implementation
```

---

## Sub-prompt 2F.4 — Cleanup and regression guard

```
This is the final sub-prompt. 2F.1-2F.3 are complete. Infrastructure
and data model in place; now we harden against regressions and remove
migration scaffolding.

Same workflow: explicit exit codes, do NOT commit.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Remove deprecated shims
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Any functions from the old single-file render API that were kept
#[deprecated] during 2F.2: remove them now unless a specific reason
justifies keeping them. Document any kept shims in phase_2_fix_report.md.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Span::primary audit
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Span::primary exists for tests and single-file/sentinel use. Production
code should use Span::new with a real FileId.

Audit:
    grep -rn "Span::primary" crates/ | grep -v tests/ | grep -v "src/tests/"

For each PRODUCTION-code hit:
  - If genuinely single-file/sentinel, document with a comment why
    PRIMARY is correct.
  - Otherwise, replace with Span::new(real_file_id, ...). Trace where
    the FileId should come from.

Ensure the doc comment of Span::primary warns production callers:
    "Intended for tests and sentinel spans. Production code should use
     Span::new with a real FileId from the SourceMap."

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — SourceFile correctness checks
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Re-verify edge cases in line_starts (add tests if not already covered
in 2F.1):

  Empty file → line_starts == vec![0]; line_col(0) → (1, 1)
  No trailing newline → last line included
  "\r\n" counted as a single break (\n is separator)
  UTF-8 multi-byte chars: column is byte index, not grapheme index;
    document this.
  BOM (0xEF 0xBB 0xBF) at start: decide and document. For v1, leaving
    it in is acceptable.
  Very long lines: no u32 overflow (Span already u32-bounded).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Regression guard — real stdio.h smoke test
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Re-run Phase 4 smoke tests under the new Span format:

  tests/lit/sema/headers_smoke.c          — still passes
  tests/lit/sema/headers_smoke_extended.c — still passes
  tests/lit/sema/realworld.c              — still passes

Add new regression tests for multi-file diagnostic rendering:

regression_user_error_reports_user_file:
  A file #includes <stdio.h> and has a DELIBERATE type error on line 5
  (e.g., `int x = "hello";` which Phase 4 post-4.7.3 catches).
  Run the full pipeline. Assert:
    - At least one error-level diagnostic
    - Its primary span.file is the user file's FileId (NOT stdio.h)
    - Rendered output contains the user file's name, not "stdio.h"

regression_macro_error_emits_backtrace:
  Define a macro in a header that produces a type error on expansion.
  Assert rendered diagnostic contains "in expansion of macro" and
  both the macro's invocation site and expansion site.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Documentation
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Update CLAUDE.md. Under "Architecture Rules", add:

  "Every Span carries FileId and ExpansionId. Production code that
   constructs Spans must thread both through from the SourceMap and
   ExpansionTable emitted by the preprocessor. Tests may use
   Span::primary for single-file scenarios."

Create docs/diagnostics.md (or equivalent):

  Pipeline:
    source files    → SourceMap (FileId-indexed)
    preprocessor    → ExpansionTable (ExpansionId-indexed)
    every token / AST node span → (FileId, byte range, ExpansionId)
    renderer        → walks both tables for multi-file, backtrace-aware
                      diagnostics

  Include a paragraph on adding a new diagnostic: get span from the AST
  or token, pass to Diagnostic::error().span(...), renderer handles file
  lookup and backtrace rendering.

Create phases/phase_2_fix_report.md:
  - Summary of what changed
  - Test count before and after (across all four sub-prompts)
  - Performance numbers (full pipeline Test A before vs after)
  - Ariadne multi-file rendering outcome — worked, workarounds used
  - Known follow-ups deferred to later phases
  - Verdict: FIX COMPLETE / PARTIAL (explain)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 6 — Final gate
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

cargo build --all                                        → exit 0
cargo test --all                                         → all pass
cargo clippy --all-targets --all-features -- -D warnings → exit 0
cargo fmt --all -- --check                               → exit 0

Optionally, if bench infrastructure exists:
  Full pipeline Test A under 10ms release.

Verify phases/phase_2_fix_report.md exists and is non-trivial.

STOP. Do not commit. Report:
  - All four gate exit codes
  - Final project test count (delta since start of Phase 2 fix)
  - Perf numbers (before vs after)
  - Any follow-ups deferred to later phases
```

---

## Risks and what to watch

- **Ariadne multi-file API** is the single biggest technical risk in 2F.2 and confirmed-or-not in 2F.3. Ariadne advertises multi-file rendering. If it proves awkward for our specific layering, Claude Code should report the friction rather than silently falling back. Fallback plan: a small custom renderer that emits per-file headers with spans.

- **Expansion tree memory:** A complex `.c` file can produce thousands of `ExpansionFrame` entries. `ExpansionId` is u32 → 4B frames max. `ExpansionFrame` is ~40-50 bytes → 100K frames ~5MB. Comfortable upper bound.

- **Test migration scale:** ~1325 tests. Many construct `Span` inline. `sed`-based bulk rewrite + compile-error-driven cleanup is the pragmatic path. Budget real hours for this in 2F.2, not minutes.

- **Don't commit partial 2F.2.** It touches four crates. Either everything compiles and tests pass, or nothing commits. Partial commits poison history.

- **Circular dependency in 2F.3 SECTION 1:** If `ExpansionId` lives in `forge_preprocess`, `forge_lexer` can't import it (preprocess already depends on lexer). Solution: `ExpansionId` in `forge_diagnostics` (the recommended default). Or a new `forge_span` crate (heavier, less recommended).

---

## After 2F.4 completes

- 4 commits on main, one per sub-prompt (each committed after you review the report)
- Span format stable for Phase 5 and beyond
- Multi-file diagnostics render with proper file names
- Macro expansion backtraces render
- `phases/phase_2_fix_report.md` exists and documents the change
- Ready to start Phase 5 IR lowering

Expected total project test count: ~1325 → ~1380-1420.
