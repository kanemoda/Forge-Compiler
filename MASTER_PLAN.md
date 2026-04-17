# FORGE — Master Plan

> A novel C17 compiler featuring e-graph optimization, verified passes, energy-aware codegen, and fine-grained incrementality.

---

## Vision

Forge is an industrial-strength C17 compiler built from scratch in Rust. It targets x86-64, and is designed to compile real-world C codebases (SQLite, Redis, curl, etc.) with correctness guarantees and optimization strategies that no existing compiler offers together.

**Four differentiators, in priority order:**

1. **E-graph optimizer** — equality saturation for optimization; explores the entire rewrite space instead of fragile phase-ordered passes
2. **Verified passes (Alive2-style)** — every optimization can be mechanically proven correct via SMT solving
3. **Fine-grained incrementality** — recompile only what changed, down to the function level
4. **Energy-aware codegen** — schedule instructions and select code sequences to minimize energy consumption, not just cycles

---

## Development Environment

| Item | Detail |
|------|--------|
| Primary machine | Ryzen 3600, 16 GB RAM, Ubuntu 24.04 |
| Secondary machine | MacBook Air M4 (early development) |
| Language | Rust (stable toolchain) |
| Build system | Cargo workspaces |
| CI | GitHub Actions (Linux x86-64) |
| Testing | `cargo test` + custom lit-style test runner |
| AI workflow | Claude Code (Opus 4.6 max effort) writes all code; Opus 4.6 extended reviews progress |

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│                     forge CLI                        │
├─────────────────────────────────────────────────────┤
│  Source file (.c)                                    │
│       │                                              │
│       ▼                                              │
│  ┌──────────┐   ┌────────────────┐                  │
│  │  Lexer   │──▶│  Preprocessor  │                  │
│  └──────────┘   └───────┬────────┘                  │
│                         ▼                            │
│              ┌──────────────────┐                    │
│              │  Parser (RD)     │                    │
│              │  → AST           │                    │
│              └────────┬─────────┘                    │
│                       ▼                              │
│              ┌──────────────────┐                    │
│              │ Semantic Analysis│                    │
│              │ Type Checking    │                    │
│              └────────┬─────────┘                    │
│                       ▼                              │
│              ┌──────────────────┐                    │
│              │  AST → Forge IR  │                    │
│              │  (SSA-based)     │                    │
│              └────────┬─────────┘                    │
│                       ▼                              │
│              ┌──────────────────┐                    │
│              │  E-Graph         │                    │
│              │  Optimizer       │                    │
│              │  (egg-based)     │                    │
│              └────────┬─────────┘                    │
│                       ▼                              │
│              ┌──────────────────┐                    │
│              │ Verified Passes  │                    │
│              │ (SMT / Z3)      │                    │
│              └────────┬─────────┘                    │
│                       ▼                              │
│                       ▼                              │
│              ┌───────────────┐                       │
│              │  x86-64       │                       │
│              │  Backend      │                       │
│              └───────┬───────┘                       │
│                      ▼                               │
│                 ┌─────────┐                          │
│                 │  ELF    │                          │
│                 │ Linker  │                          │
│                 └─────────┘                          │
└─────────────────────────────────────────────────────┘
```

---

## Crate Structure (Cargo Workspace)

```
forge/
├── Cargo.toml              (workspace root)
├── CLAUDE.md               (Claude Code project instructions)
├── MASTER_PLAN.md
├── phases/                  (planning docs — not compiled)
│
├── crates/
│   ├── forge_cli/           Phase 0  — CLI entry point
│   ├── forge_driver/        Phase 0  — orchestration, pipeline
│   ├── forge_diagnostics/   Phase 0  — error reporting (ariadne-based)
│   ├── forge_lexer/         Phase 1  — tokenization
│   ├── forge_preprocess/    Phase 2  — C preprocessor
│   ├── forge_parser/        Phase 3  — recursive descent → AST
│   ├── forge_sema/          Phase 4  — semantic analysis, types
│   ├── forge_ir/            Phase 5  — SSA-based IR
│   ├── forge_egraph/        Phase 6  — e-graph optimizer
│   ├── forge_codegen/       Phase 7  — shared codegen infrastructure
│   ├── forge_x86_64/        Phase 7  — x86-64 machine code
│   ├── forge_verify/        Phase 8  — Alive2-style SMT verification
│   ├── forge_incr/          Phase 9  — incremental compilation
│   └── forge_energy/        Phase 10 — energy-aware scheduling
│
├── tests/
│   ├── lit/                 lit-style .c test files
│   ├── conformance/         C17 conformance tests
│   └── integration/         compile-and-run tests
│
└── docs/
    ├── architecture.md
    ├── ir_spec.md
    └── contributing.md
```

---

## Phases

Each phase has a dedicated planning document in `phases/`. Each document contains:
- Precise scope and deliverables
- Technical design decisions
- Acceptance criteria (what "done" looks like)
- Exact prompts to give Claude Code
- Testing strategy
- Dependencies on prior phases

### Phase 0 — Project Scaffolding & CI
**File:** `phases/phase_00_scaffolding.md`
**Duration estimate:** 1–2 days
**Goal:** Cargo workspace, CI, error reporting crate, CLI skeleton, test harness.

### Phase 1 — Lexer
**File:** `phases/phase_01_lexer.md`
**Duration estimate:** 3–5 days
**Goal:** Full C17 tokenizer. All keywords, punctuators, literals (int, float, char, string with escape sequences), identifiers, line tracking. No preprocessor directives yet — those are handled as raw tokens.

### Phase 2 — Preprocessor
**File:** `phases/phase_02_preprocessor.md`
**Duration estimate:** 7–14 days
**Goal:** Full C17 preprocessor. `#include`, `#define` (object-like and function-like with variadic), `#if`/`#ifdef`/`#elif`/`#else`/`#endif`, `#pragma`, `#error`, `#line`, token pasting (`##`), stringification (`#`), `__FILE__`, `__LINE__`, `__DATE__`, `__TIME__`, `_Pragma`. This is one of the hardest phases.

### Phase 3 — Parser & AST
**File:** `phases/phase_03_parser.md`
**Duration estimate:** 10–18 days
**Goal:** Hand-written recursive descent parser producing a complete C17 AST. Declarations, statements, expressions (with correct precedence via Pratt parsing), type specifiers, struct/union/enum, function definitions, initializer lists, _Generic, _Static_assert, compound literals, designated initializers.

### Phase 4 — Semantic Analysis & Type System
**File:** `phases/phase_04_sema.md`
**Duration estimate:** 10–18 days
**Goal:** Type checking, implicit conversions, scope resolution, symbol tables, constant expression evaluation, lvalue/rvalue analysis, storage class validation, linkage resolution, incomplete type handling, VLA support, `_Alignof`/`_Alignas`.

### Phase 5 — Forge IR
**File:** `phases/phase_05_ir.md`
**Duration estimate:** 7–12 days
**Goal:** SSA-based intermediate representation. Basic blocks, phi nodes, typed instructions, function/module structure. AST-to-IR lowering. IR printer and parser (for testing). Designed from day one to feed into the e-graph optimizer.

### Phase 6 — E-Graph Optimizer
**File:** `phases/phase_06_egraph.md`
**Duration estimate:** 14–25 days
**Goal:** Integration with the `egg` crate. Forge IR → e-graph → optimized IR extraction. Rewrite rules for: algebraic simplification, constant folding/propagation, strength reduction, dead code elimination, common subexpression elimination, inlining heuristics. Cost function for extraction. This is the core technical differentiator.

### Phase 7 — Code Generation (x86-64)
**File:** `phases/phase_07_codegen.md`
**Duration estimate:** 18–30 days
**Goal:** Forge IR → machine instructions → ELF object files. Register allocation (linear scan or graph coloring), instruction selection (tree-pattern matching), stack frame layout, calling conventions (System V ABI), basic instruction scheduling. External linker invocation (system `ld` or `lld`). x86-64 target.

### Phase 8 — Verified Passes (Alive2-Style)
**File:** `phases/phase_08_verify.md`
**Duration estimate:** 12–20 days
**Goal:** SMT-based translation validation. Each e-graph rewrite rule can be verified: encode source and target patterns as SMT formulas, ask Z3 to prove equivalence or find a counterexample. Integration with the optimizer so verification can run on every rewrite during development/testing.

### Phase 9 — Incremental Compilation
**File:** `phases/phase_09_incremental.md`
**Duration estimate:** 10–18 days
**Goal:** Content-addressable caching of compilation artifacts. Function-level dependency tracking. When a source file changes, re-lex/parse/analyze only affected functions, reuse cached IR and codegen for the rest. Persistent on-disk cache.

### Phase 10 — Energy-Aware Code Generation
**File:** `phases/phase_10_energy.md`
**Duration estimate:** 20–35 days
**Goal:** Instruction scheduling and code selection weighted by energy cost models. Per-microarchitecture energy tables (or learned models). DVFS-aware scheduling hints. This is the research-heaviest phase and is intentionally last.

### Phase 11 — Conformance, Real-World Testing & Hardening
**File:** `phases/phase_11_conformance.md`
**Duration estimate:** 14–25 days
**Goal:** Pass the GCC torture tests (C subset). Successfully compile and run SQLite, Redis, and curl test suites. Bug fixing, edge case handling, standards conformance polishing.

### Phase 12 — Packaging, Documentation & Ecosystem
**File:** `phases/phase_12_release.md`
**Duration estimate:** 7–14 days
**Goal:** Man pages, `--help` polish, `forge` binary distribution (cargo install, .deb, homebrew tap), LSP foundation (optional), contributor documentation, benchmarks vs GCC -O2 and Clang -O2.

---

## Estimated Total Timeline

| Phase | Optimistic | Realistic | Pessimistic |
|-------|-----------|-----------|-------------|
| 0 — Scaffolding | 1 day | 2 days | 3 days |
| 1 — Lexer | 3 days | 5 days | 8 days |
| 2 — Preprocessor | 7 days | 12 days | 20 days |
| 3 — Parser | 10 days | 16 days | 25 days |
| 4 — Sema | 10 days | 16 days | 25 days |
| 5 — IR | 7 days | 10 days | 15 days |
| 6 — E-Graph | 14 days | 22 days | 35 days |
| 7 — Codegen | 18 days | 28 days | 40 days |
| 8 — Verify | 12 days | 18 days | 28 days |
| 9 — Incremental | 10 days | 15 days | 22 days |
| 10 — Energy | 20 days | 30 days | 45 days |
| 11 — Conformance | 14 days | 22 days | 35 days |
| 12 — Release | 7 days | 12 days | 18 days |
| **Total** | **~133 days** | **~208 days** | **~319 days** |

With Claude Code doing all the writing: realistic estimate is **7–10 months** of active development. This assumes daily engagement, not calendar time.

---

## Key Dependencies

```
Phase 0 ──▶ Phase 1 ──▶ Phase 2 ──▶ Phase 3 ──▶ Phase 4
                                                    │
                                                    ▼
                                                Phase 5 (IR)
                                                    │
                                          ┌─────────┼─────────┐
                                          ▼         ▼         ▼
                                      Phase 6    Phase 7   Phase 9
                                     (E-Graph)  (Codegen) (Incremental)
                                          │         │
                                          ▼         ▼
                                      Phase 8    Phase 10
                                     (Verify)   (Energy)
                                          │         │
                                          └────┬────┘
                                               ▼
                                          Phase 11 (Conformance)
                                               │
                                               ▼
                                          Phase 12 (Release)
```

---

## Development Principles

1. **Test-first, always.** Every phase builds tests before or alongside features. Claude Code should never move forward without green tests.
2. **Modularity over cleverness.** Each crate has a clean public API. Crates communicate through well-defined IR types, not shared mutable state.
3. **Diagnostics are not afterthoughts.** Every error the compiler produces must have a span, a message, and ideally a suggestion. Users judge compilers by their error messages.
4. **Document as you go.** Each crate's `lib.rs` has a module-level doc comment explaining the design. Complex algorithms get inline comments.
5. **CI never breaks.** Every commit must pass `cargo test`, `cargo clippy`, and `cargo fmt --check`.

---

## File Index

| File | Purpose |
|------|---------|
| `MASTER_PLAN.md` | This document — overall roadmap |
| `CLAUDE.md` | Instructions for Claude Code across the whole project |
| `phases/phase_00_scaffolding.md` | Project setup, CI, diagnostics |
| `phases/phase_01_lexer.md` | C17 tokenizer |
| `phases/phase_02_preprocessor.md` | C preprocessor |
| `phases/phase_03_parser.md` | Recursive descent parser & AST |
| `phases/phase_04_sema.md` | Semantic analysis & type system |
| `phases/phase_05_ir.md` | Forge IR design & AST lowering |
| `phases/phase_06_egraph.md` | E-graph optimizer (egg) |
| `phases/phase_07_codegen.md` | x86-64 backend |
| `phases/phase_08_verify.md` | Alive2-style verified passes |
| `phases/phase_09_incremental.md` | Incremental compilation |
| `phases/phase_10_energy.md` | Energy-aware code generation |
| `phases/phase_11_conformance.md` | Testing & hardening |
| `phases/phase_12_release.md` | Packaging & documentation |
