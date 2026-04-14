# Phase 12 — Packaging, Documentation & Ecosystem

**Depends on:** Phase 11 (Conformance)
**Unlocks:** Public release
**Estimated duration:** 7–14 days

---

## Goal

Polish Forge for public release. Create documentation, packaging, benchmarks, and everything needed for developers to adopt Forge.

---

## Deliverables

1. **Documentation:** architecture guide, user manual, IR specification
2. **Packaging:** cargo install, .deb package, Homebrew tap
3. **Benchmarks:** compile time and output quality vs GCC and Clang
4. **README:** compelling project README for GitHub
5. **Website/landing page** (optional): single-page site explaining Forge's differentiators
6. **Contributing guide:** how to add optimization rules, extend the backend, run tests

---

## Claude Code Prompts

### Prompt 12.1 — Documentation

```
Create comprehensive documentation for the Forge compiler.

1. docs/architecture.md:
   - High-level architecture diagram (text-based)
   - Description of each crate and its role
   - Data flow through the pipeline: source → tokens → AST → typed AST → IR → e-graph → machine code → ELF
   - Key design decisions and rationale

2. docs/user_guide.md:
   - Installation instructions (cargo install, from source, binary download)
   - Basic usage: forge build file.c, forge check file.c
   - Compiler flags: -O0, -O1, -O2, -Oenergy, --target, -I, -o
   - Diagnostics: how to read error messages
   - Comparison with GCC/Clang flags

3. docs/ir_spec.md:
   - Complete Forge IR specification
   - All opcodes with types and semantics
   - Text format grammar
   - Examples of common patterns in IR

4. docs/energy_model.md:
   - Energy optimization approach
   - Cost model sources
   - When and why to use -Oenergy

5. docs/egraph_optimization.md:
   - Explain the e-graph approach for a general audience
   - List all rewrite rules with explanations
   - How to add a new rule and verify it
   - Comparison with traditional pass-based optimization

6. CONTRIBUTING.md:
   - How to set up the dev environment
   - How to run tests
   - How to add a new optimization rule (with step-by-step)
   - How to add an instruction to the backend
   - Code style and PR guidelines
```

### Prompt 12.2 — README and benchmarks

```
Create a compelling README.md for the Forge GitHub repository.

Include:
1. Project description and logo/banner (text-based for now)
2. The four differentiators with brief explanations
3. Quick start:
   ```
   cargo install forge-cc
   forge build hello.c -o hello
   ./hello
   ```
4. Feature comparison table: Forge vs GCC vs Clang (what Forge does that they don't)
5. Build status badges (CI)
6. Conformance status: "Passes X% of GCC torture tests, successfully compiles SQLite"
7. Links to documentation

Create a benchmarks directory and script:

1. benchmarks/compile_time.sh:
   - Compile SQLite with Forge, GCC -O2, and Clang -O2
   - Measure and compare compile times
   - Output a table

2. benchmarks/code_quality.sh:
   - Compile a set of small benchmark programs with each compiler
   - Run each and measure execution time
   - Compare output binary sizes
   - Output a table

3. benchmarks/energy.sh:
   - Compare -O2 vs -Oenergy output
   - Show instruction distribution differences
   - Report estimated energy savings

Document results in docs/benchmarks.md with honest analysis — it's fine if Forge is slower than GCC/Clang at this stage. Focus on what's unique.
```

### Prompt 12.3 — Packaging and distribution

```
Set up packaging and distribution for Forge.

1. cargo install support:
   - Ensure `forge_cli` is publishable to crates.io
   - Add metadata to Cargo.toml: description, license (MIT/Apache-2.0), repository, categories, keywords
   - Verify `cargo install forge-cc` works from a clean environment

2. Binary releases:
   - Create .github/workflows/release.yml:
     - Triggers on Git tags (v*)
     - Builds binaries for: Linux x86-64, Linux AArch64, macOS x86-64, macOS AArch64
     - Creates GitHub Release with binaries attached
   - Use cross-compilation for AArch64 builds

3. Debian package:
   - Create a scripts/build-deb.sh that builds a .deb package
   - Installs forge to /usr/local/bin/forge
   - Includes man page

4. Homebrew tap (for macOS):
   - Create a separate repo: homebrew-forge
   - Formula that downloads the binary release
   - `brew tap yourname/forge && brew install forge`

5. Man page:
   - Create a forge.1 man page documenting CLI usage
   - Install with the package

Verify each distribution method works in a clean Docker container.
```

### Prompt 12.4 — Final polish

```
Final polish pass on the entire Forge project.

1. Code cleanup:
   - Run `cargo clippy --all-targets` and fix all warnings
   - Run `cargo fmt --all` 
   - Remove any TODO comments that were resolved
   - Add doc comments to all public types and functions
   - Remove dead code (cargo warns about this)

2. Error message audit:
   - Go through every Diagnostic::error call
   - Ensure messages are clear, specific, and helpful
   - Add suggestions where possible ("did you mean...?")
   - Compare with Rust/Clang error quality as reference

3. Performance:
   - Profile compiling SQLite (`cargo flamegraph` or perf)
   - Fix any obvious hot spots
   - Ensure memory usage stays under 1GB for large files

4. Security:
   - Ensure no path traversal in #include handling
   - Ensure no unbounded memory allocation from malicious input
   - Fuzz one more time after all changes

5. Create a v0.1.0 tag with changelog:
   - CHANGELOG.md listing all features
   - Tag as pre-release / alpha
   - Celebrate.
```

---

## Notes

- The README is the most important document. It's the first thing anyone sees. Make it concise, honest, and exciting.
- Don't oversell benchmarks. If Forge is 3x slower than Clang at compiling, say so — and explain why (new project, less optimization in the codegen) while highlighting what Forge does that Clang doesn't.
- The v0.1.0 release doesn't need to be perfect. It needs to demonstrate the concept, compile real code, and show each differentiator working.
