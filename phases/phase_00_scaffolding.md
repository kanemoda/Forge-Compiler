# Phase 0 — Project Scaffolding & CI

**Depends on:** Nothing
**Unlocks:** Phase 1 (Lexer)
**Estimated duration:** 1–2 days

---

## Goal

Set up the Cargo workspace, CI pipeline, error reporting infrastructure, CLI skeleton, and test harness. After this phase, you have a project that compiles, has CI, and can report errors beautifully — but doesn't do anything with C yet.

---

## Deliverables

1. **Cargo workspace** with initial crates: `forge_cli`, `forge_driver`, `forge_diagnostics`
2. **CI via GitHub Actions** — runs `cargo test`, `cargo clippy`, `cargo fmt --check` on every push
3. **`forge_diagnostics` crate** — wrapper around `ariadne` (or `codespan-reporting`) for beautiful error output with source spans
4. **`forge_cli` crate** — CLI argument parsing via `clap`, basic subcommands: `forge build <file.c>`, `forge check <file.c>`, `forge version`
5. **`forge_driver` crate** — placeholder pipeline that reads a .c file and returns "not implemented yet" through each stage
6. **Test harness** — a `tests/` directory structure and a basic lit-style test runner (or use `datatest-stable`)
7. **`CLAUDE.md`** at the repo root

---

## Technical Decisions

- **Error reporting:** Use `ariadne` — it produces Rust-compiler-quality error messages with colored spans, multi-line annotations, and notes. It's better than `codespan-reporting` for our purposes.
- **CLI:** Use `clap` with derive macros. Keep it simple for now.
- **Test runner:** Use `datatest-stable` for file-driven tests. Each `.c` file in `tests/lit/` will have expected output in comments (`// CHECK: ...` or `// ERROR: ...`).

---

## Acceptance Criteria

- [ ] `cargo build` succeeds with no warnings
- [ ] `cargo test` passes (even if tests are trivial)
- [ ] `cargo clippy` passes with no warnings
- [ ] `cargo fmt --check` passes
- [ ] `forge build hello.c` prints a structured "not implemented" message with the filename
- [ ] `forge version` prints version info
- [ ] GitHub Actions CI runs on push to `main` and on PRs
- [ ] `forge_diagnostics` can render a sample error with source span highlighting

---

## Claude Code Prompts

### Prompt 0.1 — Initialize the workspace

```
Create a new Rust project for the Forge C compiler. Initialize a Cargo workspace with the following structure:

forge/
├── Cargo.toml          (workspace, resolver = "2")
├── crates/
│   ├── forge_cli/      (binary crate)
│   ├── forge_driver/   (library crate)
│   └── forge_diagnostics/ (library crate)
├── tests/
│   ├── lit/            (empty dir with .gitkeep)
│   └── integration/    (empty dir with .gitkeep)
└── docs/               (empty dir with .gitkeep)

Dependencies:
- forge_cli: clap (with derive feature), forge_driver
- forge_driver: forge_diagnostics
- forge_diagnostics: ariadne

In forge_cli/src/main.rs:
- Parse CLI args with clap. Subcommands: `build` (takes a file path), `check` (takes a file path), and a `--version` flag.
- For `build` and `check`, read the file contents, pass them to forge_driver, and print the result.

In forge_driver/src/lib.rs:
- A public function `compile(filename: &str, source: &str) -> Result<(), Vec<Diagnostic>>` that currently just returns Ok(()).
- Define a basic `Diagnostic` type or re-export from forge_diagnostics.

In forge_diagnostics/src/lib.rs:
- A `Diagnostic` struct with: message (String), span (Range<usize>), severity (Error/Warning/Note), and optional labels.
- A function `render_diagnostics(source: &str, filename: &str, diagnostics: &[Diagnostic])` that uses ariadne to print them beautifully.

Make sure everything compiles with `cargo build` and passes `cargo clippy`. Add a basic test in forge_driver that calls compile() with an empty string.
```

### Prompt 0.2 — Set up CI

```
Create a GitHub Actions workflow file at .github/workflows/ci.yml for the Forge compiler project. It should:

1. Trigger on push to `main` and on all pull requests
2. Run on ubuntu-latest
3. Steps:
   - Checkout code
   - Install Rust stable toolchain with clippy and rustfmt components
   - Cache cargo registry and target directory
   - Run `cargo fmt --all -- --check`
   - Run `cargo clippy --all-targets --all-features -- -D warnings`
   - Run `cargo test --all`

Keep it simple and fast. Use `dtolnay/rust-toolchain` action for Rust setup and `Swatinem/rust-cache` for caching.
```

### Prompt 0.3 — Test harness setup

```
Set up a file-driven test harness for the Forge compiler. We want to be able to write test files like:

tests/lit/lexer/integers.c:
```
// RUN: forge check %s
// CHECK: 42
int x = 42;
```

Create a test runner in tests/integration/lit_runner.rs (or as a separate binary in the workspace) that:
1. Finds all .c files under tests/lit/
2. Reads the `// RUN:` line to determine the command (replacing %s with the file path)
3. Reads `// CHECK:` lines for expected stdout substrings
4. Reads `// ERROR:` lines for expected stderr substrings
5. Runs the command and asserts the checks pass

For now, use `datatest-stable` or a simple custom approach. Create 1-2 dummy test files that exercise the runner (they can test against `forge version` or similar for now since the compiler doesn't do anything yet).

Make sure `cargo test` runs these tests.
```

### Prompt 0.4 — Diagnostics demo

```
In forge_diagnostics, add a demonstration/test that shows the error rendering works. Create a test function that:

1. Takes this sample C source: `int main() { return 0 }`  (missing semicolon)
2. Creates a Diagnostic with severity Error, message "expected ';' after return statement", and a span pointing to the position after '0'
3. Renders it using ariadne and verifies the output contains the error message

Also add a convenience builder pattern for Diagnostic:
  Diagnostic::error("message").span(10..15).label("expected ';' here").note("every statement in C must end with a semicolon")

This will be the pattern used throughout the compiler for all error reporting.
```

---

## Notes

- Don't overthink this phase. It's plumbing. The goal is to have a working skeleton that every future phase plugs into.
- The `forge_driver` pipeline will grow: each phase adds a stage. For now it's just a passthrough.
- Make sure `forge_diagnostics` errors look *beautiful*. This is the first thing users see when something goes wrong, and it sets the tone for the whole project.
