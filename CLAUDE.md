# CLAUDE.md — Forge Compiler

## Project Overview

Forge is a C17 compiler written in Rust. It compiles standard C17 source code to native x86-64 executables. Its four differentiators are: e-graph-based optimization (using the `egg` crate), Alive2-style verified passes (using Z3), fine-grained incremental compilation, and energy-aware code generation.

## Repository Structure

```
forge/
├── crates/
│   ├── forge_cli/           — Binary: CLI entry point (clap)
│   ├── forge_driver/        — Library: compilation pipeline orchestration
│   ├── forge_diagnostics/   — Library: error reporting (ariadne)
│   ├── forge_lexer/         — Library: C17 tokenizer
│   ├── forge_preprocess/    — Library: C17 preprocessor
│   ├── forge_parser/        — Library: recursive descent parser → AST
│   ├── forge_sema/          — Library: semantic analysis & type checking
│   ├── forge_ir/            — Library: SSA-based intermediate representation
│   ├── forge_egraph/        — Library: e-graph optimizer (egg)
│   ├── forge_codegen/       — Library: shared codegen infrastructure
│   ├── forge_x86_64/        — Library: x86-64 backend
│   ├── forge_verify/        — Library: SMT verification (Z3) [optional feature]
│   ├── forge_incr/          — Library: incremental compilation
│   └── forge_energy/        — Library: energy-aware code generation
├── tests/
│   ├── lit/                 — File-driven tests (lexer, parser, sema, IR, etc.)
│   ├── integration/         — Integration tests
│   ├── run/                 — End-to-end compile-and-run tests
│   ├── torture/             — GCC torture test adaptations
│   └── regression/          — Bug regression tests
├── docs/                    — Documentation
├── phases/                  — Planning documents (not compiled)
├── MASTER_PLAN.md           — Project roadmap
└── CLAUDE.md                — This file
```

## Build & Test Commands

```bash
# Build everything
cargo build

# Build in release mode
cargo build --release

# Run all tests
cargo test --all

# Run tests for a specific crate
cargo test -p forge_lexer

# Run clippy (must pass with no warnings)
cargo clippy --all-targets --all-features -- -D warnings

# Format check
cargo fmt --all -- --check

# Format fix
cargo fmt --all

# Run the compiler
cargo run -- build input.c -o output
cargo run -- check input.c
cargo run -- emit-ir input.c
```

## Development Rules

### Code Quality
- **Every PR must pass:** `cargo test`, `cargo clippy` (no warnings), `cargo fmt --check`
- **No `unwrap()` or `expect()` in library code** (crates other than forge_cli). Use `?` operator or return `Result`. The only exception is truly unreachable code with a comment explaining why.
- **Every public type and function has a doc comment.**
- **Every new feature has tests.** Write tests first or alongside the implementation, never after.

### Architecture Rules
- **Crates communicate through defined interfaces.** No crate reaches into another crate's internals. Everything goes through `pub` API in `lib.rs`.
- **The IR verifier runs after every transformation in debug builds.** If you create or modify IR, run the verifier.
- **Diagnostics are not optional.** Every error the compiler can produce must have: a span (source location), a clear message, and where possible, a suggestion or note.
- **Use `forge_diagnostics` for all user-facing errors.** Internal errors (bugs in the compiler) should use `panic!` with a descriptive message or return `Err` with context.

### Naming Conventions
- Crate names: `forge_<component>` (snake_case)
- Types: PascalCase (`TokenKind`, `BasicBlock`, `IrType`)
- Functions: snake_case (`parse_expression`, `lower_to_ir`)
- Constants: SCREAMING_SNAKE_CASE
- Test functions: `test_<what_is_being_tested>` (descriptive names, not `test1`, `test2`)

### IR Conventions
- SSA values are named `%0`, `%1`, etc. in text format
- Basic blocks are named `entry`, `then`, `else`, `loop_header`, `loop_body`, `loop_exit`, `merge`, etc.
- Block parameters (instead of phi nodes) for SSA merge points
- All IR instructions are typed
- Opaque pointer type (`Ptr`) — no typed pointers (like modern LLVM)

### Testing Conventions
- **Unit tests:** in `src/tests.rs` or `#[cfg(test)] mod tests` within each module
- **Lit tests:** `.c` files in `tests/lit/<phase>/` with `// CHECK:` and `// ERROR:` comments
- **Run tests:** `.c` files in `tests/run/` that compile and execute, checking exit codes or stdout
- **Regression tests:** minimal reproducers for fixed bugs, named `issue_NNN.c`

## Target Information

### x86-64 (LP64)
- char: 1 byte, short: 2, int: 4, long: 8, long long: 8, pointer: 8
- Calling convention: System V AMD64 ABI
- Integer args: RDI, RSI, RDX, RCX, R8, R9
- Float args: XMM0-XMM7
- Return: RAX (int), XMM0 (float)
- Callee-saved: RBX, RBP, R12-R15
- Stack aligned to 16 bytes at call

## Key Dependencies

| Crate | Used For |
|-------|----------|
| `clap` | CLI argument parsing |
| `ariadne` | Beautiful error diagnostics |
| `egg` | E-graph data structure and equality saturation |
| `object` | ELF object file emission |
| `z3` | SMT solving for verification (optional feature) |
| `blake3` | Content hashing for incremental compilation |
| `bincode` / `serde` | IR serialization for caching |

## Current Phase

Check MASTER_PLAN.md for the current development phase and phases/<current_phase>.md for detailed instructions and prompts.

## Common Patterns

### Adding a new IR opcode
1. Add variant to `Opcode` enum in `forge_ir`
2. Add encoding in `forge_egraph` language definition
3. Add instruction selection in `forge_x86_64`
4. Add to IR printer and parser
5. Add to IR verifier type checking
6. Add tests at each level

### Adding a new optimization rule
1. Add the rewrite rule in `forge_egraph/src/rules.rs`
2. Add verification in `forge_verify` (prove it correct with Z3)
3. Add a lit test showing the optimization fires
4. Run the full test suite to check for regressions

### Adding a new diagnostic
```rust
use forge_diagnostics::Diagnostic;

Diagnostic::error("expected ';' after expression")
    .span(expr.span.end..expr.span.end + 1)
    .label("expected ';' here")
    .note("every expression statement in C must end with a semicolon")
```

## Environment Notes
- Primary dev machine: Ubuntu 24.04, x86-64, Ryzen 3600, 16GB RAM
- Z3 required for verification: `sudo apt install libz3-dev`
- System headers needed for #include tests: `sudo apt install libc6-dev`

## Test Organization Rules (MANDATORY)

### Where to put tests

**NEVER create `#[cfg(test)] mod tests { }` blocks inside production source files.**

All tests go in `src/tests/` submodule structure:

```
crates/<crate>/
├── src/
│   ├── lib.rs          ← contains only: #[cfg(test)] mod tests;
│   ├── whatever.rs     ← production code ONLY, zero test code
│   └── tests/
│       ├── mod.rs      ← declares submodules
│       ├── helpers.rs  ← shared test utilities (NOT a test file)
│       ├── feature_a.rs
│       └── feature_b.rs
```

### Rules

1. **New crate?** Create `src/tests/` from the start. Never inline.
2. **New feature?** Add tests to the matching `src/tests/<feature>.rs` file. If no matching file exists, create one and add it to `mod.rs`.
3. **Shared helpers** (lex-and-assert, run-preprocessor, etc.) go in `src/tests/helpers.rs`. Other test files import via `use super::helpers::*;`.
4. **Test file imports:** Use `use crate::{...};` for the crate's own types. Use `use super::helpers::*;` for shared helpers.
5. **Exception:** Tests that MUST access private fields/methods (not `pub` or `pub(crate)`) may stay inline in that specific source file. Max ~15 tests per inline block. If it grows beyond that, make the needed items `pub(crate)` and move tests out.
6. **External integration tests** (`crates/<crate>/tests/*.rs`) are for subprocess/CLI tests only (like system_headers.rs). Unit tests always go in `src/tests/`.
7. **Lit tests** stay in `tests/lit/<phase>/`. These are file-driven tests, not Rust unit tests.

### Naming

- Test files: named after the FEATURE, not the source file. `macros.rs` not `preprocessor_tests.rs`.
- Test functions: `snake_case_describing_behavior`. `stringify_escapes_backslash` not `test_3`.
- helpers.rs is NOT a test file — no `#[test]` functions in it.
