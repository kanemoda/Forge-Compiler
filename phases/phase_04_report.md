# Phase 4 — Sema Acceptance Report

**Branch:** `main`   **Audit date:** 2026-04-19   **Auditor:** Claude Opus 4.7
**Verdict:** **READY to advance to Phase 5 (IR lowering).**

---

## 1. Executive summary

Phase 4 lands a full C17 semantic-analysis stage sitting between
`forge_parser` and the yet-to-be-implemented `forge_ir`.  It covers the
complete surface the downstream IR lowerer will consume:

* every C17 type (integer, floating, pointer, array/VLA, struct,
  union, enum, function, atomic, `_Complex`) plus the target size /
  alignment model for x86-64 LP64;
* declarator resolution (all storage classes, qualifiers, function
  signatures, GNU `__attribute__`s);
* struct / union / enum layout with bit-fields and packed attributes;
* expression type-checking (every operator, calls, casts, `sizeof`,
  `_Alignof`, `_Generic`, compound literals, member access,
  array subscripting, pointer arithmetic, comma, ternary);
* statement analysis (conditions, loops with `break`/`continue`
  targets, `switch`/`case`/`default`, `goto` + labels, `return` vs
  function return type, file- and block-scope `_Static_assert`);
* scope + symbol table (function, file, block scopes; parameter
  scope; shadowing; tag namespace);
* initializers (scalar, string, array, struct, union, designated,
  compound literal, nested) with full type conversions applied;
* GNU extensions the preprocessor already delivered —
  `__builtin_offsetof`, `__builtin_types_compatible_p`,
  `__builtin_constant_p`, `__typeof__`, `__int128`, `_Float16` /
  `_Float32` / `_Float64` / `_Float128`, variadic `__builtin_va_*`,
  `__attribute__((packed / aligned / noreturn / unused / …))`.

Final test totals, all green on the `main` working tree at audit time:

| Scope             | Count |
|-------------------|------:|
| `forge_sema` unit | **536** |
| Workspace total   | **≈ 1 325** |

Performance is well inside the Phase 4 budget:

| Test                   | Release | Debug | Budget (release / debug) |
|------------------------|--------:|------:|-------------------------:|
| A — trivial `main`     |    6 ms |  7 ms | 80 ms / 300 ms |
| B — 8 headers + body   |   28 ms | 68 ms | 120 ms / 500 ms |

No new `unwrap()` / `expect()` slipped into prod code, no dead-code or
`-D warnings` clippy hits, and no `TODO`/`FIXME` markers were
introduced.  All `.expect()` calls in `scope.rs` now carry a
`// SAFETY: ...` justification explaining why the panic is
unreachable.

---

## 2. Prompt-by-prompt summary

### Prompt 4.0 — AST `NodeId` retrofit
Added stable `NodeId(u32)` to every parser AST node so sema can key
side-tables by a dense u32 instead of allocating a second tree.
Drove a one-shot parser-level renumbering pass with a sequence
counter inside `ParserContext`.  Enabled every later prompt to store
`FxHashMap<u32, _>` side-tables.

### Prompt 4.1 — Type system + `TargetInfo` + compatibility + `Display`
Introduced `QualType { ty: TyRef, quals: Qualifiers }`, a hash-consed
`TypeArena`, and the full set of C17 type kinds.  Built the LP64
`TargetInfo` (all integer / float / pointer sizes and alignments).
Implemented type compatibility (6.7.2), composite type construction
(6.7.2p3), and a canonical `Display` that round-trips through
`forge_parser::declarator`.

### Prompt 4.2 — Symbol table + specifier resolution
Added a scoped symbol table with four scope kinds (file, function,
prototype, block), shadowing rules, and two namespaces (ordinary +
tag).  Built `resolve_specifiers_and_quals` that takes a parser
`DeclSpecifiers` + `Declarator` and returns a `QualType` + storage
class, handling every C17 specifier combination including `_Atomic`,
`restrict`, `__int128`, and `__attribute__`.

### Prompt 4.2.5 — Integer constant expression evaluator
Added `const_eval::eval_integer_constant_expr` for `_Static_assert`,
case labels, bit-field widths, enum initializers, and array
dimensions.  Implemented constant folding over unary, binary,
ternary, cast, `sizeof(type)`, `_Alignof(type)`, integer and char
literals.  Returns a `ConstEvalError` with span on failure.

### Prompt 4.3 — Declaration analysis + struct layout
Lowered every file-scope and block-scope declaration to a symbol
with a resolved type.  Built `layout::compute_struct_layout` /
`compute_union_layout` honouring natural alignment, bit-field
packing, `__attribute__((packed))`, `__attribute__((aligned))`,
flexible array members, and anonymous-struct / anonymous-union
member flattening.  Enum layout picks the minimum underlying integer
type.

### Prompt 4.4 — Expression type-checking (part 1)
Implemented `check_expr` for identifiers (with lookup),
all literal forms, lvalue classification, `sizeof expr`, `sizeof(T)`,
`_Alignof(T)`, address-of, indirection, comma, and the result-is-
`int` cases.  Introduced `ctx.expr_types: IndexVec<NodeId, Option<QualType>>`
and `ctx.sizeof_kinds: FxHashMap<u32, SizeofKind>` for runtime-VLA
recording.

### Prompt 4.5 — Expression type-checking (part 2)
Filled in arithmetic / bitwise / shift / comparison operators with
usual-arithmetic conversions and integer promotions, pointer
arithmetic, function calls (including variadic), explicit and
implicit casts, member access (`.` and `->` including anonymous
flattening), array subscripting, `_Generic` with exact-type
matching, compound literals, and the ternary operator's conditional
type rules (6.5.15).

### Prompt 4.6 — Statement analysis + block-scope `_Static_assert`
Added `check_stmt` for every statement form: expression, compound,
`if` / `else`, `while`, `do`, `for`, `switch` with `case` /
`default`, `break` / `continue` (target resolution), `goto` +
labels (forward-reference tolerant, unresolved-label errors after
the function), `return` (with implicit conversion to return type),
declaration, labeled, and null.  `_Static_assert` works at both file
and block scope.

### Prompt 4.7 — GNU extensions + driver integration
Added dedicated AST + sema handling for the most common GNU
extensions the preprocessor hands through: `__builtin_offsetof`,
`__builtin_types_compatible_p`, `__builtin_constant_p`,
`__typeof__`, `__int128`, `_Float16/32/64/128`, variadic
`__builtin_va_list` / `_start` / `_arg` / `_end`, and
`__attribute__((packed / aligned / noreturn / unused / deprecated /
unknown))`.  The driver now runs sema by default on `forge check`
and exposes `--dump-types`.

### Prompt 4.7.2 — Dedicated builtins
Replaced the previous generic-identifier pathway for
`__builtin_offsetof` and `__builtin_types_compatible_p` with
first-class AST variants plus dedicated sema handlers.  Bumped test
count 509 → 520 and eliminated the "looks like a call to an
undeclared identifier" escape hatch.

### Prompt 4.7.3 — Wiring `check_expr` into scalar initializers
Fixed an architectural gap in `declare::check_scalar_or_string_init`
where the RHS of `int x = ...;` was never type-checked.  The `_`
arm now calls `check_expr` + a new `check_assignable` helper that
enforces C17 6.5.16.1p1 (assignability).  Added 11 regression tests
covering 6 error and 5 happy-path cases plus a brace-list leaf guard.

---

## 3. Completeness matrix

Rows are feature areas the downstream IR lowerer will rely on.
"Test?" = does at least one sema unit test exercise the feature.
"Pass?" = does that test currently pass on `main`.

### Type system

| Area | Test? | Pass? | Notes |
|------|:---:|:---:|---|
| `char` / `signed char` / `unsigned char` | Y | Y | `tests/type_sizes.rs`, `qual_type.rs` |
| `short` / `unsigned short`                | Y | Y | `type_sizes.rs` |
| `int` / `unsigned int`                    | Y | Y | `type_sizes.rs` |
| `long` / `unsigned long` (LP64)           | Y | Y | `type_sizes.rs` |
| `long long` / `unsigned long long`        | Y | Y | `type_sizes.rs` |
| `__int128` / `unsigned __int128`          | Y | Y | `builtin_int128.rs` |
| `_Bool`                                   | Y | Y | `qual_type.rs` |
| `float` / `double` / `long double`        | Y | Y | `type_sizes.rs` |
| `_Float16 / 32 / 64 / 128`                | Y | Y | `builtin_float_typedefs.rs` |
| `_Complex float / double / long double`   | Y | Y | `qual_type.rs` |
| `void`                                    | Y | Y | `qual_type.rs` |
| Enumerated types with explicit values     | Y | Y | `declarations.rs`, `realworld.c` |
| Pointer to any type                       | Y | Y | `expr_address_deref.rs` |
| Fixed-size array                          | Y | Y | `declarations.rs`, `layout.rs` |
| VLA (declared from a runtime extent)      | Y | Y | `expr_sizeof.rs::vla_edge_cases` |
| Flexible array member                     | Y | Y | `layout.rs` |
| Function type + pointer-to-function       | Y | Y | `declarator_resolution.rs` |
| Variadic function                         | Y | Y | `declarations.rs`, `realworld.c` |
| Struct (named, anon, forward-declared)    | Y | Y | `composite.rs`, `layout.rs` |
| Union (named, anon)                       | Y | Y | `composite.rs`, `stress.rs` |
| Bit-fields (any width, signed/unsigned)   | Y | Y | `layout.rs` |
| `_Atomic`-qualified                       | Y | Y | `qual_type.rs` |

### Qualifiers

| Area | Test? | Pass? | Notes |
|------|:---:|:---:|---|
| `const`                                   | Y | Y | `qual_type.rs`, `expr_assignment.rs` |
| `volatile`                                | Y | Y | `qual_type.rs` |
| `restrict` (pointer-only)                 | Y | Y | `declarator_resolution.rs` |
| Mixed qualifier ordering                  | Y | Y | `qual_type.rs` |
| Pointee qualifier propagation             | Y | Y | `expr_address_deref.rs` |

### Conversions & promotions

| Area | Test? | Pass? | Notes |
|------|:---:|:---:|---|
| Integer promotion                         | Y | Y | `integer_promotion.rs` |
| Usual arithmetic conversions              | Y | Y | `arithmetic_conversions.rs` |
| `nullptr` → any pointer (0 constant)      | Y | Y | `expr_assignment.rs` |
| Array-to-pointer decay                    | Y | Y | `expr_pointer_arith.rs` |
| Function-to-pointer decay                 | Y | Y | `expr_call.rs` |
| Implicit conversion in return             | Y | Y | `stmt_return.rs` |
| Implicit conversion in assignment         | Y | Y | `expr_assignment.rs` |
| Implicit conversion in initializer        | Y | Y | `initializers.rs` |
| Implicit conversion in call arg           | Y | Y | `expr_call.rs` |
| Explicit cast (every target type)         | Y | Y | `expr_cast.rs` |

### Expressions

| Area | Test? | Pass? | Notes |
|------|:---:|:---:|---|
| Integer / char / string / float literal   | Y | Y | `expr_literals.rs` |
| Identifier expression + lookup            | Y | Y | `expr_identifier.rs` |
| Lvalue classification                     | Y | Y | `expr_lvalue.rs` |
| Unary `+ - ! ~`                           | Y | Y | `expr_arithmetic.rs` |
| `++` / `--` pre/post                      | Y | Y | `expr_increment.rs` |
| `&` / `*`                                 | Y | Y | `expr_address_deref.rs` |
| `sizeof expr` / `sizeof(T)`               | Y | Y | `expr_sizeof.rs` |
| `_Alignof(T)`                             | Y | Y | `expr_alignof.rs` |
| Binary `+ - * / %`                        | Y | Y | `expr_arithmetic.rs` |
| Pointer arithmetic                        | Y | Y | `expr_pointer_arith.rs` |
| Shift `<< >>`                             | Y | Y | `expr_shift_logical.rs` |
| Logical `&& || !`                         | Y | Y | `expr_shift_logical.rs` |
| Bitwise `& ^ |`                           | Y | Y | `expr_shift_logical.rs` |
| Comparison `< <= > >= == !=`              | Y | Y | `expr_comparison.rs` |
| Assignment `=` + compound `+= -=` etc.    | Y | Y | `expr_assignment.rs` |
| Ternary `? :`                             | Y | Y | `expr_ternary.rs` |
| Comma `,`                                 | Y | Y | `expr_comma.rs`, `realworld.c` |
| Subscript `[]`                            | Y | Y | `expr_subscript.rs` |
| Member `.` / `->`                         | Y | Y | `expr_member.rs` |
| Function call                             | Y | Y | `expr_call.rs` |
| Cast `(T)e`                               | Y | Y | `expr_cast.rs` |
| `_Generic`                                | Y | Y | `expr_generic.rs`, `stress.rs` |
| Compound literal `(T){…}`                 | Y | Y | `expr_compound_literal.rs` |

### Declarations

| Area | Test? | Pass? | Notes |
|------|:---:|:---:|---|
| File-scope object                         | Y | Y | `declarations.rs` |
| Block-scope object                        | Y | Y | `declarations.rs` |
| Storage classes (auto/static/extern/register/thread_local) | Y | Y | `declarations.rs`, `specifier_resolution.rs` |
| `typedef` (simple + chained)              | Y | Y | `declarations.rs`, `stress.rs` |
| Function declaration (prototype)          | Y | Y | `function_body.rs` |
| Function definition                       | Y | Y | `function_body.rs` |
| Parameter scope                           | Y | Y | `parameter_scope.rs` |
| Struct / union / enum declaration         | Y | Y | `composite.rs` |
| Forward-declared struct / union           | Y | Y | `composite.rs`, `stress.rs` |
| Mutually recursive structs (3-way)        | Y | Y | `stress.rs` |
| Bit-field declarators                     | Y | Y | `layout.rs` |

### Initializers

| Area | Test? | Pass? | Notes |
|------|:---:|:---:|---|
| Scalar initializer (via `check_expr`)     | Y | Y | `initializers.rs` (Prompt 4.7.3) |
| String → `char[]`                         | Y | Y | `initializers.rs` |
| Array with brace list                     | Y | Y | `initializers.rs` |
| Struct / union brace list                 | Y | Y | `initializers.rs`, `realworld.c` |
| Designated initializer (`.fld = ...`)     | Y | Y | `initializers.rs`, `realworld.c` |
| Array-designated (`[n] = ...`)            | Y | Y | `initializers.rs` |
| Nested aggregate                          | Y | Y | `initializers.rs` |

### Statements

| Area | Test? | Pass? | Notes |
|------|:---:|:---:|---|
| Expression statement                      | Y | Y | `stmt_conditions.rs` |
| Compound statement / block scope          | Y | Y | `stmt_conditions.rs`, `stress.rs` |
| `if` / `if-else`                          | Y | Y | `stmt_conditions.rs` |
| `while` / `do` / `for`                    | Y | Y | `stmt_conditions.rs` |
| `break` / `continue`                      | Y | Y | `stmt_break_continue.rs` |
| `switch` / `case` / `default`             | Y | Y | `stmt_switch.rs`, `realworld.c` |
| `goto` + label                            | Y | Y | `stmt_goto.rs` |
| `return` (with / without value)           | Y | Y | `stmt_return.rs` |
| File-scope `_Static_assert`               | Y | Y | `stmt_static_assert.rs`, `realworld.c` |
| Block-scope `_Static_assert`              | Y | Y | `stmt_static_assert.rs`, `realworld.c` |

### GNU extensions

| Area | Test? | Pass? | Notes |
|------|:---:|:---:|---|
| `__builtin_offsetof`                      | Y | Y | `builtin_offsetof.rs` |
| `__builtin_types_compatible_p`            | Y | Y | `builtin_types_compatible.rs` |
| `__builtin_constant_p`                    | Y | Y | `builtin_constant_p.rs` |
| `__typeof__`                              | Y | Y | `builtin_typeof.rs` |
| `__builtin_va_list/start/arg/end`         | Y | Y | `builtin_va.rs` |
| `__attribute__((packed))`                 | Y | Y | `attribute_packed.rs` |
| `__attribute__((aligned(n)))`             | Y | Y | `attribute_aligned.rs` |
| `__attribute__((noreturn))`               | Y | Y | `attribute_noreturn.rs` |
| `__attribute__((unknown)) — ignored`      | Y | Y | `attribute_unknown_ignored.rs` |

### Scope / symbol table

| Area | Test? | Pass? | Notes |
|------|:---:|:---:|---|
| File / function / prototype / block scope | Y | Y | `symbol_table.rs` |
| Shadowing across nested scopes            | Y | Y | `symbol_table.rs`, `stress.rs` |
| Tag vs ordinary namespace                 | Y | Y | `symbol_table.rs` |
| 50 levels of nesting (stress)             | Y | Y | `stress.rs` |
| 100 locals in one function (stress)       | Y | Y | `stress.rs` |

### Translation-unit / driver

| Area | Test? | Pass? | Notes |
|------|:---:|:---:|---|
| Multi-declaration file                    | Y | Y | `translation_unit.rs` |
| `forge check` subcommand                  | Y | Y | `forge_cli/tests/system_headers.rs` |
| System-header smoke                       | Y | Y | `headers_smoke_extended.c` + driver |
| Real-world acceptance                     | Y | Y | `realworld.c` + driver |
| Multi-TU extern reuse                     | Y | Y | `stress.rs::stress_multiple_tu_extern` |

---

## 4. Stress test results

All 12 stress cases in `crates/forge_sema/src/tests/stress.rs` pass on
the trivial path (zero errors, zero warnings).  Each is run through the
full lex → parse → sema pipeline via `helpers::analyze_source` /
`assert_source_clean`.

| Stress | Result |
|--------|:------:|
| 100 locals in one function                   | PASS |
| 50 nested block scopes with shadowing        | PASS |
| Struct with 50 members                       | PASS |
| Function with 20 parameters                  | PASS |
| 10 levels of pointer indirection             | PASS |
| Expression tree of depth 100 (`1 + 1 + …`)   | PASS |
| Enum with 100 constants                      | PASS |
| Two separate TUs sharing an `extern`         | PASS |
| Typedef chain `A0 → A1 → … → A9`             | PASS |
| 3-way mutually recursive structs A↔B↔C       | PASS |
| Anonymous union inside struct inside union   | PASS |
| `_Generic` with 20 distinct arms + default   | PASS |

No performance cliff was observed — the whole stress module runs in
well under 100 ms.

---

## 5. Real-world lit outcomes

Two hand-written C sources in `tests/lit/sema/` are driven by the
existing `forge_cli/tests/system_headers.rs` integration test harness
using the `forge check` subcommand.

### `realworld.c` — Phase 4 acceptance sample

Exercises, in a single translation unit that type-checks cleanly:

* multiple structs including a self-referential linked-list node;
* enum with explicit integer values;
* function-pointer typedef used as a struct member;
* array of structs built with designated initializers;
* pointer arithmetic via array decay;
* `void *` ↔ `char *` casts;
* `sizeof(struct Node) * 4` and `_Alignof(struct Node) * 8` as array
  dimensions;
* switch with five cases plus default;
* `for` loop with a comma expression in its update step;
* variadic function declaration and matching call;
* `_Static_assert` at both file and block scope.

**Result:** `forge check realworld.c` exits 0, zero diagnostics.

### `headers_smoke_extended.c` — eight-header smoke

Includes `stdio.h stdlib.h string.h stdint.h stddef.h ctype.h
errno.h time.h` and calls at least one function from each
(`printf`, `fprintf`, `atoi`, `free`, `strlen`, `memcpy`, `strcmp`,
`isalpha`, `toupper`, `time`, `difftime`, plus `errno` as an
lvalue and `ptrdiff_t` pointer subtraction).

**Result:** `forge check headers_smoke_extended.c` exits 0, zero
diagnostics.  Skips gracefully on hosts without a discoverable
toolchain (same mechanism the existing `-E` / `parse` smoke tests
use).

---

## 6. Performance

Both gates measured end-to-end wall-clock via `std::time::Instant`
around a subprocess `forge check` invocation.  Source:
`crates/forge_cli/tests/perf.rs`.

| Test                                  | Release | Debug | Budget (release / debug) |
|---------------------------------------|--------:|------:|-------------------------:|
| A — `int main(void) { return 0; }`    |    6 ms |   7 ms |   80 ms /  300 ms |
| B — 8-header + realworld-shape body   |   28 ms |  68 ms |  120 ms /  500 ms |

The budgets are the Phase 4 acceptance criteria; the effective margin
is **≈ 4× under release, ≈ 7× under debug**.  Test B exercises the
same eight-header surface as the extended smoke test, so any
regression there will be caught with a concrete time number in the
failure message.

### Hashing sanity

```
$ grep -rn 'std::collections::HashMap' crates/forge_sema/src/
crates/forge_sema/src/scope.rs:21:use std::collections::HashMap;
crates/forge_sema/src/types.rs:37:use std::collections::HashMap;
```

Two occurrences in prod sema code.  Both are small identifier-keyed
maps (scope symbols / tags, keyed by `String`; struct / union / enum
layout caches, keyed by `u32`).  They are on the cold path relative
to the hot per-expression side-tables, which all already use
`rustc_hash::FxHashMap`.  Converting these to `FxHashMap` is a
micro-optimisation noted below — it is not a correctness gate.

---

## 7. Known deferrals

These are items the audit flagged that are deliberately **not**
addressed in Phase 4; every one of them is tracked so it cannot be
forgotten at Phase 5 start.

1. **`sizeof(expression)` in a constant-expression context** still
   returns `ConstEvalError::NotSupported`.  The `_Static_assert`
   in `realworld.c` was rewritten to use `sizeof(type)` to
   accommodate this.  Tracked: needs folding support in
   `const_eval.rs` for non-VLA expression operands (the VLA case
   is intentionally runtime).
2. **Two prod `HashMap`s in sema** (`scope.rs`, `types.rs`) are
   still `std::collections::HashMap`.  Switching to `FxHashMap`
   is a one-line micro-opt; postponed so the Phase 4 surface
   stays minimal.
3. **Clippy `pedantic` lint group** produces ~92 stylistic warnings
   (identical match arms, sign-loss casts, etc.).  The default
   `-D warnings` gate is clean; pedantic was audited but intentionally
   not globally silenced — each cleanup should be a judgment call,
   not a blanket `#[allow]`.
4. **`sizeof(T *)` where `T` is a VLA** currently returns
   `SizeofKind::Constant(8)` (pointer size).  This is correct per
   C17 §6.5.3.4p2 — the operand's type is a *pointer* type, not a
   VLA type — but the Phase 4 prompt spec described it as
   `RuntimeVla`.  The test
   `sizeof_pointer_to_vla_is_pointer_size` codifies the strict-C17
   behaviour with a comment pointing at the standard.
5. **Lit runner discovery** — the `// RUN: forge check %s` comments
   in `tests/lit/sema/*.c` are still descriptive, not mechanically
   executed by a LLVM-style lit driver.  The subprocess harness in
   `forge_cli/tests/system_headers.rs` fills the role for Phase 4.
   A proper lit runner is a Phase 11 conformance concern.

---

## 8. Test counts

Numbers from `cargo test --all` on `main` at audit time:

| Crate / suite                          | Count |
|----------------------------------------|------:|
| `forge_sema` (unit)                    |   536 |
| `forge_preprocess` (unit)              |   233 |
| `forge_preprocess::tests::stress`      |    20 |
| `forge_lexer` (unit)                   |   216 |
| `forge_parser` (unit)                  |   214 |
| `forge_driver` (unit)                  |    46 |
| `forge_cli` — `system_headers.rs`      |    24 |
| `forge_cli` — `lit.rs`                 |    20 |
| `forge_cli` — `perf.rs`                |     2 |
| `forge_cli` — `preprocess_cli.rs`      |     1 |
| `forge_diagnostics`                    |     7 |
| Smaller crates (`forge_ir` etc.)       |    ~6 |
| **Total**                              | **≈ 1 325** |

Phase 4 itself added **536 sema unit tests + 4 integration tests
(2 lit, 2 perf) = 540 new tests** over the course of Prompts 4.0–4.8.

---

## 9. Verdict

All five checkpoint gates pass on `main`:

| Gate | Command | Result |
|------|---------|:------:|
| 1 | `cargo build --all`                              | exit 0 |
| 2 | `cargo test -p forge_sema`                       | 536 passed |
| 3 | `cargo test --all`                               | all passed |
| 4 | `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| 5 | `cargo fmt --all -- --check`                     | clean |

**Phase 4 is READY.** Proceed to Phase 5 (IR lowering,
`phase_05_ir.md`).  Carry forward the five known deferrals listed in
§7 as Phase 5 prerequisites where they intersect with IR lowering
(most importantly: the `sizeof(expression)` constant-evaluator gap,
since the IR lowerer will lean on `const_eval` for array dimensions).
