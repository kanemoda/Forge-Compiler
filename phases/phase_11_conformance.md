# Phase 11 — Conformance, Real-World Testing & Hardening

**Depends on:** Phase 7 (Codegen), ideally all prior phases
**Unlocks:** Phase 12 (Release)
**Estimated duration:** 14–25 days

---

## Goal

Prove that Forge works on real software. Pass established test suites, compile real-world C projects (SQLite, Redis, curl), fix every bug discovered, and harden the compiler for production use.

---

## Deliverables

1. **GCC torture test results** — pass rate on the C-language subset
2. **SQLite compilation** — compile and pass SQLite's test suite
3. **Redis compilation** — compile and pass basic Redis tests
4. **curl compilation** — compile and link successfully
5. **Bug fix log** — every bug found and fixed during this phase
6. **Compiler stability** — no crashes on any valid C17 input, graceful handling of invalid input

---

## Target Test Suites

### GCC Torture Tests
- ~2000 small C programs testing language features and edge cases
- Located in GCC source tree under `gcc/testsuite/gcc.c-torture/execute/`
- Download and adapt as a Forge test suite
- Realistic initial target: 70-80% pass rate, then iterate to 95%+

### SQLite
- Single-file C program (~250K lines after amalgamation)
- Extensive test suite (`sqlite3 test`)
- Exercises: complex macros, preprocessor, structs, function pointers, varargs, unions
- This is the gold standard for C compiler testing

### Redis
- Multi-file C project
- Exercises: networking code, data structures, signal handling, fork/exec
- Requires: correct struct layout, pointer arithmetic, varargs, system calls

### curl
- Complex build system (autoconf/cmake)
- Exercises: heavy preprocessor use, platform detection macros, function pointer tables
- Even compiling (not necessarily passing all tests) is a significant achievement

---

## Hardening Checklist

- [ ] No compiler crashes on any GCC torture test (even ones we can't compile should produce error messages, not panics)
- [ ] Every Forge-internal panic replaced with proper error handling
- [ ] Fuzzing: run `cargo fuzz` on the lexer, preprocessor, and parser to find crash bugs
- [ ] Memory: no memory leaks in the compiler itself (Rust helps here, but check for arena/cache leaks)
- [ ] Diagnostics: every error message has a source span, a clear message, and ideally a fix suggestion
- [ ] Large file handling: can compile a 100K-line C file without running out of memory

---

## Claude Code Prompts

### Prompt 11.1 — GCC torture test integration

```
Set up the GCC torture test suite as a Forge test suite.

1. Create a script (scripts/fetch_torture_tests.sh) that:
   - Clones the GCC repository (shallow, just the test directory)
   - Copies gcc/testsuite/gcc.c-torture/execute/*.c to tests/torture/
   - Strips tests that require GCC extensions we don't support (mark them as expected failures)

2. Create a test runner (tests/torture_runner.rs or a script) that:
   - For each .c file in tests/torture/:
     a. Runs `forge build test.c -o test_binary`
     b. If compilation fails: record as "compile fail"
     c. If compilation succeeds: run the binary, check exit code 0
     d. If binary crashes or returns non-zero: record as "runtime fail"
   - Generates a summary: PASS / COMPILE_FAIL / RUNTIME_FAIL / EXPECTED_FAIL
   - Tracks pass rate as a percentage

3. Create an expected failures list (tests/torture/expected_failures.txt) for:
   - Tests using GCC extensions (__builtin_*, __attribute__, etc.)
   - Tests requiring features we haven't implemented yet
   - Tests relying on undefined behavior that GCC happens to handle

4. Run the test suite, record initial pass rate. File GitHub issues for categories of failures.

Goal: identify the most common failure categories and prioritize fixes.
```

### Prompt 11.2 — Bug fixing (iteration 1: preprocessor and parser)

```
Based on the torture test results, fix the most common compiler failures.

Common issues to expect and fix:

1. Preprocessor:
   - Missing GCC predefined macros (__GNUC__, __has_builtin, __has_attribute)
   - Add compatibility macros: #define __GNUC__ 4 (enough to satisfy most feature detection)
   - GNU-specific extensions in system headers: fix by adding minimal support or compatibility shims
   - Computed includes: #include MACRO_THAT_EXPANDS_TO_HEADER

2. Parser:
   - GNU statement expressions: ({ expr; expr; })
   - __typeof__ / typeof extension
   - Designated initializer ranges: [0 ... 9] = value
   - __attribute__((unused)), __attribute__((aligned(N))) — parse and ignore most attributes
   - Compound literals in global initializers
   - Empty struct (GCC extension)

3. Type system:
   - Implicit function declarations (C89 holdover, common in test code)
   - Incomplete array types in more contexts
   - Subtle integer promotion edge cases
   - Enum values as integer constants in more contexts

For each bug:
- Add a minimal reproducer test case
- Fix the issue
- Verify the fix with the reproducer
- Re-run the torture suite to confirm the pass rate improved

Target: increase pass rate by 15-20 percentage points.
```

### Prompt 11.3 — Bug fixing (iteration 2: codegen and runtime failures)

```
Fix runtime failures from the torture tests — programs that compile but produce wrong results.

These are the hardest bugs because they indicate codegen or optimization errors.

Debugging strategy:
1. For each failing test, compile with -O0 (no optimization). If it passes at -O0 but fails at -O2, the bug is in the optimizer.
2. Compare Forge's IR output with expected behavior (manually trace through the IR).
3. Use the IR verifier to check for malformed IR.
4. Compare assembly output with GCC's output for reference.

Common codegen bugs:
- Incorrect struct layout (wrong alignment or padding)
- Wrong calling convention (argument passed in wrong register)
- Stack frame corruption (incorrect stack offset calculation)
- Integer promotion errors (sign extension vs zero extension)
- Comparison result used incorrectly (signed vs unsigned comparison)
- Shift operations with incorrect shift amount handling
- Switch statement fall-through not handled correctly

For each bug:
- Isolate to a minimal C reproducer (< 20 lines)
- Add the reproducer to tests/regression/
- Fix the bug
- Verify the fix
- Re-run the full torture suite

Target: achieve 85%+ pass rate on the torture suite.
```

### Prompt 11.4 — SQLite compilation

```
Compile and test SQLite with Forge.

1. Download the SQLite amalgamation (sqlite3.c + sqlite3.h + shell.c):
   wget https://www.sqlite.org/2024/sqlite-amalgamation-XXXXX.zip

2. Attempt to compile:
   forge build sqlite3.c -c -o sqlite3.o
   forge build shell.c -c -o shell.o
   cc sqlite3.o shell.o -o sqlite3 -ldl -lpthread -lm

3. Fix all compilation errors. Common issues with SQLite:
   - Heavy use of preprocessor feature detection (#ifdef __GNUC__, #if defined(_WIN32), etc.)
   - Uses varargs (stdarg.h)
   - Uses longjmp/setjmp
   - Complex union types
   - Extensive use of void* casting
   - Function pointer tables

4. Once it compiles, run basic SQLite tests:
   echo "CREATE TABLE t1(x); INSERT INTO t1 VALUES(42); SELECT * FROM t1;" | ./sqlite3
   Expected output: 42

5. If SQLite has its own test harness (Tcl-based), try running a subset.

6. Document all bugs found and fixed during this process.

This is a MAJOR milestone — if Forge can compile SQLite, it can compile most real-world C.
```

### Prompt 11.5 — Fuzzing and crash hardening

```
Set up fuzzing and eliminate all compiler crashes.

1. Add cargo-fuzz targets:
   - fuzz_lexer: feed random bytes to the lexer
   - fuzz_preprocess: feed random token streams to the preprocessor
   - fuzz_parser: feed random (but roughly valid) token streams to the parser
   - fuzz_full: feed random .c files through the full pipeline

2. For each fuzz target:
   - The target should return Ok/Err, NEVER panic
   - Run fuzzing for at least 1 hour per target on the Ryzen machine
   - Fix every crash found

3. Replace all unwrap() and expect() calls in library code with proper error handling:
   - Search for unwrap/expect in all crates
   - Replace with ? operator or proper error returns
   - Only keep unwrap() in test code and truly unreachable cases (with comments explaining why)

4. Ensure the compiler handles gracefully:
   - Empty files
   - Binary files (not valid C)
   - Extremely long lines (>1MB)
   - Deeply nested constructs (1000 levels of parentheses)
   - Files with millions of tokens (from massive headers)

5. Add CI step to run a short fuzzing session (5 minutes) on every PR.
```

---

## Notes

- This phase is iterative. The prompt numbers are starting points — you'll cycle between running tests, identifying failures, and fixing bugs many times.
- Keep a running document (docs/conformance_log.md) tracking pass rates over time. This is both motivating and useful for the project README.
- Real-world compilation is where all the corner cases emerge. The spec says one thing, but real code often relies on implementation-defined behavior or minor extensions.
- GCC attribute parsing is a pragmatic necessity. Don't implement the semantics — just parse and discard `__attribute__((...))` to avoid syntax errors.
