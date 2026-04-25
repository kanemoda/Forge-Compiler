# Phase 5 (v3) — Forge IR

**Depends on:** Phase 4 (Semantic Analysis), Phase 2 fix (multi-file Span + macro tracking)
**Unlocks:** Phase 6 (E-Graph), Phase 7 (Codegen), Phase 9 (Incremental)
**Estimated duration:** 8-10 days, 8 sub-prompts (5.0 → 5.7)

**This document supersedes the master plan's `phases/phase_05_ir.md` and v2.**

## Changelog (v2 → v3)

After v2 review with Gemini, seven concrete changes:

1. **Memory order enum on Load/Store** (§5.6, §6) — Plain/Volatile/Atomic(ordering). C17 `volatile` correctness was missing from v2; without this, e-graph optimization would silently break memory-mapped I/O. Atomic ordering field is reserved for Phase 6+ but the enum shape is fixed now.
2. **Escape analysis broadened in Prompt 5.0** (§11) — address-taken now includes any store-destination where `&local` is the value being stored. Catches `s.field = &x;`, `*pp = &x;`, `arr[i] = &x;` patterns that v2's "pointer assignment" rule missed.
3. **Alignment check in verifier** (§12) — when a static GEP chain produces a load/store with constant offset, the offset must be aligned for the access type. Catches misaligned access bugs early.
4. **`restrict` qualifier representation** (§5, §13) — Function parameter flag in v3; per-instruction alias metadata deferred to Phase 6 e-graph design.
5. **UB handling policy stated** (§13) — default behavior is wraparound on signed overflow. `nsw`/`nuw` flags deferred until Phase 6 e-graph policy is decided. Forge's identity is "sane compiler", not "aggressive UB exploitation."
6. **Pointer address space note** (§4) — `Ptr` is opaque; address-space variants deferred to Phase 10+ when non-flat-memory targets enter scope.
7. **Sema layout invariant explicit** (§3 Decision 2) — IR never computes struct/array layout; it only consumes pre-computed offsets from `StructLayout`. Stated as a hard invariant for the verifier and reviewers.

---

## 1. Goal

Design and implement Forge's intermediate representation: a typed, SSA-based IR sitting between the typed AST (forge_sema) and machine code (forge_codegen, Phase 7). The IR is **designed for e-graph optimization first** — every architectural choice is evaluated against "does this make Phase 6 easier or harder?"

---

## 2. Deliverables

1. **`forge_ir` crate** — IR types, builder, printer, parser, verifier
2. **Address-taken analysis** in `forge_sema` (Prompt 5.0) — prerequisite for the smart memory model
3. **AST-to-IR lowering** (`forge_lower` module inside `forge_ir`, or sibling crate)
4. **IR text format** (printer + parser, round-trip stable)
5. **IR verifier** (mandatory in debug, runs after every transformation)
6. **Driver integration** (`forge --emit-ir file.c`)
7. **`phases/phase_05_report.md`** — final audit, completeness matrix, perf, deferrals

---

## 3. Architectural decisions (the smart trio)

Three foundational choices, each with rationale. These are decided BEFORE prompt-writing because they cascade through every later decision.

### Decision 1 — Memory model: Cranelift-style direct SSA (no mem2reg pass)

**What:** Locals whose address is never taken are lowered directly as SSA values. Locals whose address IS taken (via `&x`, passed to a function as `T*`, etc.) are lowered as `stack_alloc` + `load`/`store`.

**Why not LLVM-style alloca-everywhere:**
- LLVM lowers every local as `alloca`, then runs `mem2reg` to promote scalars. This is a 2-pass solution.
- For e-graph: more `load`/`store` instructions = more side-effecting nodes = harder rewriting.
- Doing it right at lowering time means the IR is closer to canonical from day one.

**Why this works:** Phase 4 sema has all the type and use information. We need ONE additional analysis: "is this variable's address taken anywhere in its scope?" That's Prompt 5.0.

**Address-taken includes (any case where `&local` reaches "outside the local's own scope"):**

The unifying rule: **if `&local` appears as a value that gets stored, returned, or passed somewhere the local's scope can't see, the local is address-taken.** Specific patterns:

- `&local` used directly as the value of an assignment: `p = &x;`, `s.field = &x;`, `*pp = &x;`, `arr[i] = &x;`, `g_ptr = &x;` (global)
- Local passed to a function as a pointer parameter: `f(&x)`, `f(x)` where `x` is an array (array-to-pointer decay)
- Local returned as a pointer: `return &x;` (illegal but must be flagged)
- Local cast to a pointer of any kind: `(void*)&local`, `(char*)&local`
- Local's address written into a memory location of any kind via any compound construct: `s = (struct S){.p = &x};`, `arr[0] = &x;`

This is essentially **escape analysis** — does the address escape the local's stack frame? If yes → memory-resident. If no → SSA.

**Address-taken does NOT include (these stay as SSA):**
- Read-only use of value: `int y = x;`
- Use in arithmetic or comparison: `x + 1`, `x > 0`
- Passing by value: `f(x)` where `x` is `int` (value copy, no address)
- Use as `sizeof` operand: `sizeof x` (sizeof doesn't evaluate)

### Decision 2 — GEP: Byte offset arithmetic (no type-aware GEP)

**What:** GEP takes a base pointer and a byte offset (compile-time integer or runtime value), returns a new pointer. No type information, no field names, no element indices.

**Example:**
```
%struct_ptr = stack_alloc 24                    ; struct of 24 bytes
%field_ptr  = gep %struct_ptr, +16              ; pointer to field at offset 16
%field_val  = load.i32 %field_ptr               ; load i32 from there
```

**Compare to LLVM-style GEP:**
```llvm
%field_ptr = getelementptr inbounds %S, ptr %p, i32 0, i32 2   ; second field of struct S
```

**Why byte offsets:**
- E-graph rewrites can reason about pointer arithmetic algebraically (`gep(gep(p, a), b)` → `gep(p, a + b)`)
- No need for IR-level struct definitions — keeps type system minimal (10 types instead of 50)
- Sema layout already computed all offsets; lowering just emits the constant
- Codegen (Phase 7) is simpler — no GEP type interpretation

**Tradeoff:** lose alias analysis hints that LLVM's typed GEP provides. We accept this — Forge's alias analysis will be span-based and explicit, not GEP-derived.

**Hard invariant — layout is sema's job, not IR's:**

IR never computes struct/array layout. It only consumes pre-computed offsets and sizes from Phase 4 sema's `StructLayout` and `Type::size_of()`. Specifically:

- Lowering reads `member.offset` and emits `gep %ptr, +<offset>`
- Lowering reads `member.size` and emits `load.<ir_type> %ptr` with size determined by IrType
- Lowering reads `struct_layout.alignment` for `stack_alloc` align argument
- Lowering reads `struct_layout.is_packed` to skip alignment-aware emission

This invariant is enforced by code review and by the verifier (the alignment check from §12 fires whenever IR-level offsets contradict the layout sema would have produced). If the IR ever needs to know "where is field `f` in struct S?", that's a code smell — sema should have answered that question and the answer should already be encoded in the IR as a constant.

**Consequence:** if Phase 5 lowering hits a struct member access and the corresponding `StructLayout` is missing or incomplete, that's a Phase 4 bug, not a Phase 5 bug. Lowering emits a panic with a NodeId reference; reviewers fix it in sema, not in lowering.

### Decision 3 — Switch: Native opcode (not branch chain)

**What:** A `switch` C statement lowers to a single `Switch` IR opcode with a list of (value, target_block) pairs and a default target.

```
switch.i32 %v, default(merge), [
  0 => block_a
  1 => block_b
  5 => block_c
]
```

**Why native:**
- E-graph treats Switch as a single opaque control-flow node — no rewrites cross switch boundaries needlessly
- Codegen (Phase 7) decides between jump table, binary search, or branch chain based on case density. This decision shouldn't be hardcoded in IR.
- Equivalent representation lets verifier check for duplicate case values, which is a sema property worth asserting again at IR time.

**Tradeoff:** Phase 7 must implement switch lowering to assembly. ~200 lines of work. Worth it.

---

## 4. Type system

**Final list (10 types):**

```rust
pub enum IrType {
    I1,   // boolean (used by icmp/fcmp results, branch conditions)
    I8, I16, I32, I64,
    F32, F64,
    Ptr,  // opaque pointer, like LLVM's ptr
    Void, // function return type only; no Void values
}
```

**Excluded deliberately:**
- `Aggregate` — no struct/array types in IR. Sema computed all sizes/offsets; IR sees only bytes and offsets.
- `Vector` types — Phase 7 codegen / Phase 10 energy work. Defer.
- `I128` — Phase 4 approximates `__int128` as `i64`; defer.
- `F16`, `F128` — defer with the rest of `_FloatN` precision work.

**Convention:** opcodes that operate on numeric values are typed (`iadd.i32`, `fmul.f64`); opcodes that operate on pointers do not specify pointee type (just `load.i32 %p`).

**Address spaces (deferred):** `Ptr` is opaque — a single pointer type covers all uses. Targets like embedded MCUs (program/data memory split) and GPGPU (global/shared/local) need address-space variants on `Ptr`. We do NOT model them now. When Phase 10+ targets such an architecture, `Ptr` will become `Ptr(AddressSpace)` with a `Default` variant matching today's behavior. Document this as a known shape change.

---

## 5. Opcode set

Grouped by category. Each opcode lists arity, result type, and a "pure?" flag for e-graph eligibility.

### 5.1 Constants (pure)

| Opcode | Operands | Result | Pure |
|--------|----------|--------|------|
| `iconst` | (i64 value, IrType) | typed value | ✓ |
| `fconst` | (f64 value, IrType) | typed value | ✓ |
| `null_ptr` | () | Ptr | ✓ |

### 5.2 Integer arithmetic (pure)

| Opcode | Operands | Result | Pure |
|--------|----------|--------|------|
| `iadd`, `isub`, `imul` | (a: T, b: T) | T (same int type) | ✓ |
| `sdiv`, `udiv` | (a: T, b: T) | T | ✓ |
| `srem`, `urem` | (a: T, b: T) | T | ✓ |
| `band`, `bor`, `bxor` | (a: T, b: T) | T | ✓ |
| `bnot`, `ineg` | (a: T) | T | ✓ |
| `shl`, `lshr`, `ashr` | (a: T, b: T) | T | ✓ |

Note: `lshr` (logical) and `ashr` (arithmetic) are distinct; sema picks based on signedness.

### 5.3 Float arithmetic (pure)

| Opcode | Operands | Result | Pure |
|--------|----------|--------|------|
| `fadd`, `fsub`, `fmul`, `fdiv` | (a: F, b: F) | F | ✓ |
| `fneg` | (a: F) | F | ✓ |

(No `frem` — C standard `fmod` is a library call, not an opcode.)

### 5.4 Comparisons (pure)

| Opcode | Operands | Result | Pure |
|--------|----------|--------|------|
| `icmp` | (CmpOp, a: T, b: T) | I1 | ✓ |
| `fcmp` | (FCmpOp, a: F, b: F) | I1 | ✓ |

`CmpOp`: `Eq, Ne, Slt, Sle, Sgt, Sge, Ult, Ule, Ugt, Uge` (10 variants — signed and unsigned)

`FCmpOp`: `Oeq, One, Olt, Ole, Ogt, Oge, Ord, Uno` (ordered/unordered)

### 5.5 Conversions (pure)

| Opcode | Operands | Result | Pure |
|--------|----------|--------|------|
| `sext` | (a: T_small, T_big) | T_big | ✓ |
| `zext` | (a: T_small, T_big) | T_big | ✓ |
| `trunc` | (a: T_big, T_small) | T_small | ✓ |
| `fpext` | (a: F32) | F64 | ✓ |
| `fptrunc` | (a: F64) | F32 | ✓ |
| `sitofp`, `uitofp` | (a: T_int, F) | F | ✓ |
| `fptosi`, `fptoui` | (a: F, T_int) | T_int | ✓ |
| `bitcast` | (a: T1, T2) | T2 (same size) | ✓ |
| `inttoptr` | (a: i64) | Ptr | ✓ |
| `ptrtoint` | (a: Ptr) | i64 | ✓ |

### 5.6 Memory (mostly NOT pure)

| Opcode | Operands | Result | Pure |
|--------|----------|--------|------|
| `stack_alloc` | (size: i64, align: i64) | Ptr | ✗ |
| `load` | (ptr: Ptr, IrType result, mem_order: MemoryOrder) | result type | depends on mem_order |
| `store` | (ptr: Ptr, val: T, mem_order: MemoryOrder) | (none) | ✗ always |
| `gep` | (base: Ptr, offset: i64 or Value) | Ptr | ✓ |

`gep` is pure because it's pointer arithmetic, no memory access. `load`/`store` carry a `MemoryOrder`:

```rust
pub enum MemoryOrder {
    Plain,                       // ordinary memory access
    Volatile,                    // C17 volatile — never reorder/merge/eliminate
    Atomic(AtomicOrdering),      // C11 _Atomic — concurrency guarantees
}

pub enum AtomicOrdering {
    Relaxed,
    Acquire,
    Release,
    AcqRel,
    SeqCst,
}
```

**Purity rules:**
- `load.Plain` is **pure-with-respect-to-other-loads** — the e-graph may CSE two `load.Plain` of the same pointer if no aliasing store sits between them. (This is the standard "loads of locals can be common-subexpression-eliminated" optimization.)
- `load.Volatile` is **NEVER pure** — every volatile load is a distinct observable event, even if the pointer is the same.
- `load.Atomic(_)` is **NEVER pure** — atomic loads have synchronization side effects.
- `store.*` is always side-effecting regardless of memory order.

**Why this matters:** the e-graph (Phase 6) reads `Opcode::is_pure()` to decide whether to add a node to the equivalence graph. A volatile load wrongly classified as pure would let e-graph eliminate redundant volatile reads — silently breaking memory-mapped I/O, signal handlers, and `setjmp`. The `MemoryOrder` field is the gatekeeper.

For Phase 5 v1: lowering only emits `MemoryOrder::Plain` and `MemoryOrder::Volatile`. Atomic ordering is reserved in the type but not produced (Phase 4 sema accepts `_Atomic` as a qualifier but lowers as plain — full atomic semantics deferred).

### 5.7 Control flow (NOT pure, terminators only)

| Opcode | Operands | Result | Pure |
|--------|----------|--------|------|
| `jump` | (target: Block, args: Vec<Value>) | (none) | ✗ |
| `branch` | (cond: I1, true_target: Block, true_args, false_target: Block, false_args) | (none) | ✗ |
| `switch` | (val: T_int, default: Block, default_args, cases: Vec<(i64, Block, args)>) | (none) | ✗ |
| `return` | (val: Option<Value>) | (none) | ✗ |
| `unreachable` | () | (none) | ✗ |

`unreachable` is for `_Noreturn` function calls and unreachable-after-return cleanup. Codegen emits a trap.

### 5.8 Other (mixed purity)

| Opcode | Operands | Result | Pure |
|--------|----------|--------|------|
| `call` | (callee: FunctionRef, args: Vec<Value>) | T | ✗ (assumed) |
| `select` | (cond: I1, true_val: T, false_val: T) | T | ✓ |

`call` is assumed side-effecting unless the callee is annotated pure (Phase 6 may add this). `select` is the ternary equivalent — both branches eagerly evaluated, no control flow.

**Total: 50-ish opcodes.** Master plan estimated 30. The extra ones (separate `lshr`/`ashr`, `bitcast`, `inttoptr`/`ptrtoint`, `select`, `unreachable`) all serve specific purposes that pay off downstream.

---

## 6. NodeId → Value mapping (debug provenance)

Every IR `Instruction` carries an optional source NodeId from the AST it lowered from:

```rust
pub struct Instruction {
    pub result: Option<Value>,
    pub opcode: Opcode,
    pub ty: IrType,
    pub operands: Vec<Operand>,
    pub mem_order: Option<MemoryOrder>, // Some only for Load/Store
    pub span: Span,                      // From Phase 2 fix — multi-file aware
    pub source_node: Option<NodeId>,     // AST provenance, Phase 4 NodeId
}
```

**Why:**
- Verifier error messages: "Value %5 used in block %3 but defined in unreachable block %7 (lowered from `x = 5;` at main.c:12:5)"
- Phase 6 e-graph rewrite proofs: "this rewrite preserved the AST node's type because Phase 4 recorded `expr_types[node_id] == X`"
- Phase 8 verified passes: Alive2-style equivalence checking refers back to source semantics
- Phase 10 energy attribution: "this hot loop came from these 3 source lines"

**Cost:** `Option<NodeId>` is 8 bytes per instruction. A typical TU has ~10K instructions → 80KB. Negligible.

### 6.1 `restrict` qualifier on function parameters

C99/C17's `restrict` qualifier promises that within a pointer's scope, no other pointer aliases its target. This enables more aggressive optimization (the compiler can assume two `restrict` pointers don't overlap).

Forge's IR carries `restrict` as a flag on Function parameters:

```rust
pub struct Function {
    pub name: String,
    pub params: Vec<FunctionParam>,
    pub return_type: IrType,
    pub blocks: Vec<BasicBlock>,
    pub entry: Block,
    pub next_value: u32,
}

pub struct FunctionParam {
    pub value: Value,
    pub ty: IrType,
    pub is_restrict: bool,           // C restrict qualifier
}
```

**Per-instruction alias metadata is deferred to Phase 6** (e-graph). The reason: alias analysis decisions affect rewrite legality, and Phase 6's design will determine whether we use TBAA-style metadata, span-based partitioning, or a custom approach. Hardcoding a representation now may force a refactor later.

For Phase 5 v1: just thread the parameter flag from sema. Phase 6 will read it during e-graph construction.

---

## 7. Span integration (post-2F.4)

Phase 2 fix landed `Span { file: FileId, start: u32, end: u32, expanded_from: ExpansionId }`. IR uses this directly:

- Every Instruction has a `span: Span` (NOT `Option<Span>` — every instruction has provenance, even if it's the Span of the surrounding statement)
- Lowering inherits the span of the AST node being lowered
- Block parameters inherit the span of the predecessor's branch instruction
- Synthesized control flow (e.g., implicit return for `void` functions) uses the closing `}` span

**Verifier rendering:**
```
error: value %5 used before definition
   ┌─ main.c:12:5
12 │     int y = x + 1;
   │             ^^^^^ %5 used here
   ├─ main.c:8:9
 8 │         break;
   │         ^^^^^ predecessor block's terminator skipped %5's defining block
```

This is just `render_diagnostics` from forge_diagnostics. Verifier emits standard `Diagnostic` objects.

---

## 8. Pure vs side-effecting (e-graph readiness)

Each `Opcode` declares its purity at the type level:

```rust
impl Opcode {
    pub fn is_pure(&self) -> bool {
        match self {
            Opcode::Iadd | Opcode::Isub | Opcode::Imul
            | Opcode::Sdiv | Opcode::Udiv | ...
            | Opcode::Iconst | Opcode::Fconst
            | Opcode::Sext | Opcode::Zext | ...
            | Opcode::Gep
            | Opcode::Select
            => true,
            
            Opcode::Load | Opcode::Store | Opcode::StackAlloc
            | Opcode::Call
            | Opcode::Jump | Opcode::Branch | Opcode::Switch
            | Opcode::Return | Opcode::Unreachable
            => false,
        }
    }
}
```

E-graph (Phase 6) only adds pure instructions to the equivalence graph. Side-effecting instructions stay in their original block sequence. This avoids the entire class of bugs where e-graph rewriting accidentally reorders memory operations.

---

## 9. Block parameters (NOT phi nodes)

Master plan got this right. Block parameters are explicit at every block entry and explicit at every jump/branch/switch site. **No `Phi` opcode exists.** If lowering hits a case it can't express with block params, that's a bug in lowering, not a fallback to phi.

```
func @max(%a: i32, %b: i32) -> i32 {
  entry:
    %cmp = icmp.sgt %a, %b
    branch %cmp, then(%a), else(%b)
  then(%result: i32):
    return %result
  else(%result: i32):
    return %result
}
```

Each block lists its parameters in parens; each jump passes args. The "return %result" works because both `then` and `else` blocks parameterize on `%result`.

---

## 10. SSA construction algorithm — Braun et al. "Simple SSA"

The Braun 2013 paper "Simple and Efficient Construction of Static Single Assignment Form" describes an on-the-fly algorithm:

**API:**
```rust
impl FunctionBuilder {
    fn declare_var(&mut self, name: &str) -> VarId;
    fn def_var(&mut self, var: VarId, val: Value);
    fn use_var(&mut self, var: VarId, ty: IrType) -> Value;
    fn seal_block(&mut self, block: Block);
}
```

**How it works:**
- `def_var` records `(var, block) -> value` in a map.
- `use_var` looks up `(var, current_block)`. If found, returns it. If not, it walks predecessors:
  - If only one predecessor and it's sealed, recurse.
  - Otherwise add a block parameter to the current block, emit `use_var` calls for each predecessor's terminator to get the value-to-pass, and return the new block param.
- `seal_block(b)` marks `b` as having all predecessors known. Triggers any deferred parameter resolution.

**Why this over Cytron:**
- ~100 lines of code instead of ~500
- No dominator tree computation needed during lowering
- Cranelift uses a variant; the algorithm is battle-tested
- Pruned by construction (only generates block params for variables actually live across blocks)

---

## 11. Sub-prompts

### Prompt 5.0 — Address-taken analysis (Phase 4 sema extension)

**Purpose:** Add an analysis pass to `forge_sema` that records, for every local declaration, whether its address is taken anywhere in its scope. Lowering will use this to pick between SSA-direct and stack-alloc paths.

```
This is sub-prompt 5.0 — a Phase 4 sema extension required by Phase 5 IR
lowering's smart memory model. We are NOT touching IR yet; this is a sema
analysis pass.

Goal: for every local variable Symbol (StorageClass::None at function
scope), record a flag indicating whether its address is taken anywhere
in its scope.

Same workflow: explicit exit codes, do NOT commit — report and I will
commit manually.

REMINDER: every new feature must have at least one test that would fail
if the feature were removed. Test file names in the prompt are
requirements, not suggestions.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Symbol field
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Add to Symbol (in scope.rs or wherever Symbol is defined):

    pub address_taken: bool,

Default to false on construction. Only locals (StorageClass::None at
function scope) need accurate values; globals and parameters are always
"address-takeable" from a translation-unit perspective so we don't track
them — Phase 5 will treat them as memory-resident anyway.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Analysis pass
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

After expression and statement type-checking complete (i.e., late in
analyze_function_def, before the function scope pops), walk every
expression in the function body. The unifying rule:

  THE ESCAPE RULE
  If &local appears as a value that is stored anywhere, returned,
  or passed to a function, the local is address-taken.

Specifically, mark `symbol.address_taken = true` when:

  1. The expression `&local_var` appears (Unary AddressOf with
     operand resolving to a local).

  2. The expression `local_var` resolves to a local AND
     ctx.implicit_convs records an ArrayToPointer conversion on
     this expression (this catches array decay where the array's
     address escapes).

  3. The result of `&local` (or array-decayed local) ends up in
     ANY of these positions:
       - RHS of an assignment to anything: `p = &x;`,
         `s.field = &x;`, `*pp = &x;`, `arr[i] = &x;`, `g_ptr = &x;`
       - Argument to any function call (any pointer-typed parameter):
         `f(&x)`, `f(arr)` where arr is array
       - Operand of return: `return &x;`
       - Element of a compound literal initializer: `(struct S){.p = &x}`
       - Element of an aggregate initializer: `int *arr_of_p[1] = { &x };`

     IMPLEMENTATION SHORTCUT: rather than enumerate every parent context
     for each `&local` or array-decay site, you can simply walk the AST
     and mark address_taken whenever you see one of these productions
     containing a child that resolves to a local-with-address-taken
     candidate. Phase 4 sema's symbol_refs side-table tells you which
     identifiers resolve to which Symbols.

Do NOT mark address_taken for:
  - Read-only uses of value: `int y = x;`, `if (x > 0)`, `x + 1`
  - x in `x++` or `x = x + 1` (these modify the local but don't
    expose its address)
  - Passing by value: `f(x)` where x is int
  - Use as `sizeof` operand: `sizeof x` (sizeof doesn't evaluate)

Implementation hint: a recursive walker over the function body's
statements/expressions. For each Expr node, check if it's an
AddressOf or an ArrayToPointer-converted local; if yes, mark the
symbol. The "where does this address go" question is implicitly
answered because we're walking the whole body — every reachable
escape site eventually appears in the walk.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create crates/forge_sema/src/tests/address_taken.rs:

  scalar_local_used_in_arithmetic_is_not_address_taken:
      void f() { int x = 5; int y = x + 1; (void)y; }
      → Symbol for x has address_taken == false.

  scalar_local_addressed_with_ampersand_is_address_taken:
      void f() { int x = 5; int *p = &x; (void)p; }
      → Symbol for x has address_taken == true.

  scalar_local_passed_to_pointer_param_is_address_taken:
      void g(int *); void f() { int x = 5; g(&x); }
      → Symbol for x has address_taken == true (via &x).

  array_local_decayed_to_pointer_is_address_taken:
      void g(int *); void f() { int arr[10]; g(arr); }
      → Symbol for arr has address_taken == true (via ArrayToPointer
        conversion in argument).

  scalar_local_assigned_to_pointer_is_address_taken:
      void f() { int x = 5; int *p; p = &x; (void)p; }
      → x is address_taken.

  scalar_local_address_stored_into_struct_field_is_address_taken:
      struct S { int *p; };
      void f() { int x = 5; struct S s; s.p = &x; (void)s; }
      → x is address_taken (escapes through struct field).

  scalar_local_address_stored_via_double_indirection_is_address_taken:
      void f() { int x = 5; int *p; int **pp = &p; *pp = &x; (void)x; }
      → x is address_taken (escapes through *pp).

  scalar_local_address_stored_into_global_is_address_taken:
      int *g_ptr;
      void f() { int x = 5; g_ptr = &x; }
      → x is address_taken (escapes through global).

  scalar_local_returned_as_pointer_is_address_taken:
      int *f() { int x = 5; return &x; }
      → x is address_taken (escapes via return; this is also UB but
        we still flag it correctly).

  local_used_only_as_sizeof_operand_is_not_address_taken:
      void f() { int x = 5; size_t s = sizeof x; (void)s; }
      → x has address_taken == false (sizeof doesn't evaluate).

  parameter_address_taken_does_not_crash:
      void f(int x) { int *p = &x; (void)p; }
      → No crash; we don't track parameter address_taken (it's always
        treated as memory-resident).

  global_address_taken_does_not_crash:
      int g; void f() { int *p = &g; (void)p; }
      → No crash.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
CHECKPOINT
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

cargo build --all                                        → exit 0
cargo test -p forge_sema                                 → all pass
cargo test --all                                         → all pass
cargo clippy --all-targets --all-features -- -D warnings → exit 0
cargo fmt --all -- --check                               → exit 0

STOP. Do not commit. Report:
  - All five gate exit codes
  - New forge_sema test count
  - Any unexpected sema crashes encountered when adding the analysis
    (these would indicate that some path we're walking has a bug)
```

---

### Prompt 5.1 — IR data structures, types, opcodes

```
This is Prompt 5.1. Prompt 5.0 (address-taken analysis) is complete.

Goal: create the forge_ir crate with core data structures, types, and
opcodes. This sub-prompt produces the static skeleton — no builder
logic, no lowering, no parsing. Just the types that everything else
will manipulate.

Same workflow: explicit exit codes, do NOT commit.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Crate setup
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create crates/forge_ir/ with:
  Cargo.toml — depend on forge_diagnostics (for Span), forge_sema
    (for NodeId — confirm where NodeId actually lives; might be
    forge_parser); use FxHashMap from rustc-hash if any maps are needed.
  src/lib.rs — module declarations and re-exports.

Modules to create (initially most empty or minimal):
  src/types.rs       — IrType, CmpOp, FCmpOp
  src/opcode.rs      — Opcode enum + is_pure()
  src/value.rs       — Value, Block newtypes
  src/instruction.rs — Operand, Instruction
  src/block.rs       — BasicBlock
  src/function.rs    — Function, FunctionRef
  src/module.rs      — Module, Global

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Core types
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

types.rs:

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum IrType {
    I1, I8, I16, I32, I64,
    F32, F64,
    Ptr,
    Void,
}

impl IrType {
    pub fn size_bytes(&self) -> u32;       // 1,1,2,4,8 for ints; 4,8 for floats; 8 for ptr; 0 for void
    pub fn is_integer(&self) -> bool;
    pub fn is_float(&self) -> bool;
    pub fn is_scalar(&self) -> bool;       // integer or float (not Ptr/Void)
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum CmpOp {
    Eq, Ne,
    Slt, Sle, Sgt, Sge,
    Ult, Ule, Ugt, Uge,
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum FCmpOp {
    Oeq, One, Olt, Ole, Ogt, Oge,
    Ord, Uno,
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Value, Block, Operand
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

value.rs:

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct Value(pub u32);

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct Block(pub u32);

impl Value {
    pub const INVALID: Value = Value(u32::MAX);
}
impl Block {
    pub const INVALID: Block = Block(u32::MAX);
    pub const ENTRY: Block = Block(0);
}

instruction.rs:

#[derive(Clone, Debug)]
pub enum Operand {
    Value(Value),
    Block(Block),
    BlockArgs(Block, Vec<Value>),     // for jump/branch/switch destinations
    IntConst(i64),
    FloatConst(u64),                  // bit-pattern for stable comparison
    Type(IrType),
    Func(FunctionRef),
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Opcode (full list)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

opcode.rs:

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum Opcode {
    // Constants
    Iconst, Fconst, NullPtr,

    // Integer arithmetic
    Iadd, Isub, Imul, Sdiv, Udiv, Srem, Urem,
    Band, Bor, Bxor, Bnot, Ineg,
    Shl, Lshr, Ashr,

    // Float arithmetic
    Fadd, Fsub, Fmul, Fdiv, Fneg,

    // Comparisons
    Icmp, Fcmp,

    // Conversions
    Sext, Zext, Trunc, Fpext, Fptrunc,
    Sitofp, Uitofp, Fptosi, Fptoui,
    Bitcast, Inttoptr, Ptrtoint,

    // Memory
    StackAlloc, Load, Store, Gep,

    // Control flow (terminators)
    Jump, Branch, Switch, Return, Unreachable,

    // Other
    Call, Select,
}

impl Opcode {
    pub fn is_pure(&self) -> bool;       // see Section 8 of phase doc
    pub fn is_terminator(&self) -> bool; // Jump, Branch, Switch, Return, Unreachable
    pub fn name(&self) -> &'static str;  // "iadd", "load", etc.
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Instruction, BasicBlock, Function, Module
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

instruction.rs:

pub struct Instruction {
    pub result: Option<Value>,
    pub opcode: Opcode,
    pub ty: IrType,                          // result type; Void for terminators
    pub operands: Vec<Operand>,
    pub span: forge_diagnostics::Span,        // Phase 2 fix Span (FileId + ExpansionId)
    pub source_node: Option<NodeId>,          // AST provenance
}

block.rs:

pub struct BasicBlock {
    pub id: Block,
    pub params: Vec<(Value, IrType)>,
    pub instructions: Vec<Instruction>,
    // Predecessors are NOT stored here — computed on demand by the verifier.
    // This keeps blocks easy to mutate during lowering.
}

function.rs:

pub struct FunctionRef(pub u32);  // index into Module.functions

pub struct Function {
    pub name: String,
    pub params: Vec<(Value, IrType)>,
    pub return_type: IrType,
    pub blocks: Vec<BasicBlock>,
    pub entry: Block,
    pub next_value: u32,                  // for issuing new SSA values
}

module.rs:

pub struct Global {
    pub name: String,
    pub ty: IrType,
    pub initializer: Option<Vec<u8>>,    // raw bytes; None = tentative/zero-init
    pub linkage: Linkage,                 // Internal, External, etc.
    pub span: Span,
}

pub enum Linkage { Internal, External, Common }

pub struct Module {
    pub functions: Vec<Function>,
    pub globals: Vec<Global>,
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 6 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create crates/forge_ir/src/tests/ following the project's test
organization rule (each test file is a feature area, no inline #[cfg
(test)] mod tests in production source files).

Required test files:
  tests/types.rs       — IrType.size_bytes, is_integer, is_float
  tests/opcode.rs      — is_pure correctness for each opcode (one
                          test per category to cut down on size)
  tests/value.rs       — Value::INVALID, Block::INVALID, Block::ENTRY
  tests/manual_construction.rs
                       — manually construct a "main returns 42"
                          Function: entry block with iconst.i32 42
                          and return %0. No builder yet — direct
                          struct construction.

The manual_construction test serves as a smoke test: if the
data model doesn't permit constructing this trivially, the design
is broken before we even start.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
CHECKPOINT
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

cargo build --all                                        → exit 0
cargo test -p forge_ir                                   → all pass
cargo test --all                                         → all pass
cargo clippy --all-targets --all-features -- -D warnings → exit 0
cargo fmt --all -- --check                               → exit 0

STOP. Do not commit. Report all five gate exit codes, the new
forge_ir test count, and confirm the manual_construction test
exercised every field of Function/BasicBlock/Instruction (no field
went unset).
```

---

### Prompt 5.2 — Builder API + Braun SSA construction

```
This is Prompt 5.2. Prompts 5.0-5.1 are complete.

Goal: implement the FunctionBuilder API with Braun et al. "Simple SSA
Construction." This is what the lowering pass (Prompt 5.3+) will use.

Same workflow: explicit exit codes, do NOT commit.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — FunctionBuilder skeleton
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

crates/forge_ir/src/builder.rs:

pub struct FunctionBuilder {
    func: Function,
    current_block: Option<Block>,
    sealed: FxHashSet<Block>,
    incomplete_phis: FxHashMap<Block, Vec<(VarId, Value, IrType)>>,
    var_defs: FxHashMap<(Block, VarId), Value>,
    var_names: FxHashMap<VarId, String>,
    next_var: u32,
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct VarId(pub u32);

impl FunctionBuilder {
    pub fn new(name: String, params: Vec<IrType>, return_type: IrType) -> Self;
    pub fn finish(self) -> Function;

    // Block management
    pub fn create_block(&mut self) -> Block;
    pub fn switch_to_block(&mut self, block: Block);
    pub fn append_block_param(&mut self, block: Block, ty: IrType) -> Value;
    pub fn current_block(&self) -> Option<Block>;

    // Sealing
    pub fn seal_block(&mut self, block: Block);
    pub fn seal_all_remaining(&mut self);     // call at end of function
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Instruction builders
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Each method appends an Instruction to the current block, allocates a
new Value, and returns it.

Constants:
    pub fn iconst(&mut self, ty: IrType, value: i64, span: Span) -> Value;
    pub fn fconst(&mut self, ty: IrType, value: f64, span: Span) -> Value;
    pub fn null_ptr(&mut self, span: Span) -> Value;

Integer arithmetic (each takes lhs, rhs, span):
    pub fn iadd(...) -> Value;
    pub fn isub, imul, sdiv, udiv, srem, urem (...) -> Value;
    pub fn band, bor, bxor, shl, lshr, ashr (...) -> Value;
    pub fn bnot, ineg (...) -> Value;          // unary

Float arithmetic:
    pub fn fadd, fsub, fmul, fdiv (...) -> Value;
    pub fn fneg (...) -> Value;

Comparisons:
    pub fn icmp(&mut self, op: CmpOp, lhs: Value, rhs: Value, span: Span) -> Value;
    pub fn fcmp(&mut self, op: FCmpOp, lhs: Value, rhs: Value, span: Span) -> Value;

Conversions (each takes operand, target_ty, span):
    pub fn sext, zext, trunc, fpext, fptrunc (...) -> Value;
    pub fn sitofp, uitofp, fptosi, fptoui (...) -> Value;
    pub fn bitcast, inttoptr, ptrtoint (...) -> Value;

Memory:
    pub fn stack_alloc(&mut self, size: i64, align: i64, span: Span) -> Value;
    pub fn load(&mut self, ptr: Value, ty: IrType, span: Span) -> Value;
    pub fn store(&mut self, ptr: Value, val: Value, span: Span);  // no return
    pub fn gep(&mut self, base: Value, offset: Operand, span: Span) -> Value;
        // offset is Operand::IntConst for constant, Operand::Value for runtime

Other:
    pub fn select(&mut self, cond: Value, true_val: Value, false_val: Value,
                  ty: IrType, span: Span) -> Value;
    pub fn call(&mut self, func: FunctionRef, args: Vec<Value>, ret_ty: IrType,
                span: Span) -> Option<Value>;  // None for void calls

Terminators (current block must not already be terminated):
    pub fn jump(&mut self, target: Block, args: Vec<Value>, span: Span);
    pub fn branch(&mut self, cond: Value,
                  true_target: Block, true_args: Vec<Value>,
                  false_target: Block, false_args: Vec<Value>,
                  span: Span);
    pub fn switch(&mut self, val: Value,
                  default: Block, default_args: Vec<Value>,
                  cases: Vec<(i64, Block, Vec<Value>)>,
                  span: Span);
    pub fn return_val(&mut self, val: Option<Value>, span: Span);
    pub fn unreachable(&mut self, span: Span);

After any terminator is emitted, current_block becomes None (caller must
switch_to_block before emitting more instructions).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Variable tracking (Braun SSA)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub fn declare_var(&mut self, name: String) -> VarId;
pub fn def_var(&mut self, var: VarId, val: Value);
pub fn use_var(&mut self, var: VarId, ty: IrType, span: Span) -> Value;

Algorithm (Braun et al. 2013):

def_var(var, val):
    var_defs[(current_block, var)] = val

use_var(var, ty, span):
    return read_var(var, current_block, ty, span)

read_var(var, block, ty, span):
    if var_defs.contains_key((block, var)):
        return var_defs[(block, var)]
    return read_var_recursive(var, block, ty, span)

read_var_recursive(var, block, ty, span):
    if not sealed.contains(block):
        // Block has unknown predecessors — add a dummy block param
        val = append_block_param(block, ty)
        incomplete_phis[block].push((var, val, ty))
        var_defs[(block, var)] = val
        return val
    preds = predecessors_of(block)
    if preds.len() == 1:
        // Single predecessor; recurse into it
        val = read_var(var, preds[0], ty, span)
    else:
        // Multiple preds; need a real block param
        val = append_block_param(block, ty)
        var_defs[(block, var)] = val
        for pred in preds:
            pred_val = read_var(var, pred, ty, span)
            // Mutate pred's terminator to pass pred_val as the new arg
            append_arg_to_terminator(pred, block, pred_val)
        // Try to simplify: if all incoming values are the same, replace
        val = try_remove_trivial_param(block, val)
    var_defs[(block, var)] = val
    return val

seal_block(block):
    sealed.insert(block)
    for (var, val, ty) in incomplete_phis.remove(block).unwrap_or_default():
        // Re-resolve as if it were a real block-param now
        for pred in predecessors_of(block):
            pred_val = read_var(var, pred, ty, span)  // span: synthesized
            append_arg_to_terminator(pred, block, pred_val)

predecessors_of(block):
    // Computed on demand by walking all blocks' terminators and recording
    // who jumps/branches/switches to `block`.

try_remove_trivial_param(block, param):
    // If all incoming args are the same value (or refer back to param itself),
    // the param is trivial; replace uses with the unique value.
    // Not strictly required for correctness but generates cleaner IR.
    // Implementation can be a TODO for v1 if it gets hairy.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create crates/forge_ir/src/tests/builder.rs and related:

  tests/builder_basic.rs:
    builder_constructs_main_returning_42_via_api
        Use FunctionBuilder; assert resulting Function has 1 block,
        2 instructions, terminator is Return.
    builder_constructs_addition
        iadd(iconst.i32 1, iconst.i32 2); return result.
    builder_panics_if_emitting_after_terminator
        iconst, return, then attempt another iconst → panic (or error).

  tests/builder_ssa.rs:
    ssa_single_block_no_block_params
        declare_var, def_var, use_var in same block — returns same Value.
    ssa_two_blocks_no_split_no_merge
        Block A defines x, jumps to B, B uses x → no block param needed.
    ssa_diamond_introduces_block_param
        if/else writing to same var, merge block reads it → merge block
        has 1 param.
    ssa_loop_introduces_back_edge_param
        while loop modifying counter — header block has counter as param,
        body's increment passes new value back via jump.
    ssa_seal_block_resolves_incomplete_params
        Create block before all preds known; seal_block triggers
        param resolution.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
CHECKPOINT
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

cargo build --all                                        → exit 0
cargo test -p forge_ir                                   → all pass
cargo test --all                                         → all pass
cargo clippy --all-targets --all-features -- -D warnings → exit 0
cargo fmt --all -- --check                               → exit 0

STOP. Do not commit. Report all five gate exit codes, new test
count in forge_ir, and whether try_remove_trivial_param was
implemented or deferred (acceptable to defer for v1).
```

---

### Prompt 5.3 — Expression lowering

```
This is Prompt 5.3. Prompts 5.0-5.2 are complete.

Goal: implement AST expression → IR lowering. This sub-prompt handles
expressions only; statements are Prompt 5.4. Aggregates (struct/array
access) are Prompt 5.5.

Same workflow: explicit exit codes, do NOT commit.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — LoweringContext
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

In crates/forge_ir/src/lower/mod.rs (new submodule):

pub struct LoweringContext<'a> {
    pub builder: FunctionBuilder,
    pub sema: &'a SemaContext,                 // from Phase 4
    pub local_vars: FxHashMap<SymbolId, LocalKind>,
    // Add fields as needed — block stack for break/continue, etc.
}

pub enum LocalKind {
    Ssa(VarId),         // variable lives as SSA value, tracked by builder
    Memory(Value),      // variable is stack-allocated, Value is the Ptr
}

When lowering a function body, walk Symbol declarations: if
symbol.address_taken (from Prompt 5.0) is false AND it's a scalar type
(integer/float/pointer that fits in IrType), use LocalKind::Ssa.
Otherwise stack_alloc and use LocalKind::Memory.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — IrType from QualType
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Helper:

pub fn ir_type_for(qt: &QualType, target: &TargetInfo) -> IrType {
    match &qt.ty {
        Type::Void => IrType::Void,
        Type::Bool => IrType::I1,
        Type::Char { .. } => IrType::I8,
        Type::Short { .. } => IrType::I16,
        Type::Int { .. } => IrType::I32,
        Type::Long { .. } => IrType::I64,    // assuming LP64
        Type::LongLong { .. } => IrType::I64,
        Type::Float => IrType::F32,
        Type::Double | Type::LongDouble => IrType::F64,  // approximated
        Type::Pointer(_) | Type::Array { .. } | Type::Function { .. } => IrType::Ptr,
        Type::Struct(_) | Type::Union(_) => IrType::Ptr,  // aggregates always
                                                            // by reference at IR level
        Type::Enum(eid) => {
            let underlying = ctx.type_ctx.get_enum(*eid).underlying_type;
            ir_type_for(&QualType::unqualified(underlying), target)
        }
    }
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Expression lowering
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub fn lower_expr(ctx: &mut LoweringContext, expr: &Expr) -> Value;

Apply implicit conversions BEFORE returning. Read ctx.sema.implicit_convs
for the expr's NodeId; for each conversion in order, emit the corresponding
IR instruction (sext, zext, trunc, sitofp, etc.).

Per-expression handling:

Literals:
  IntLiteral(v) → iconst(ir_ty, v, span)
  FloatLiteral(v) → fconst(ir_ty, v, span)
  CharLiteral(c) → iconst(I8, c as i64, span)
  StringLiteral(s) → emit module-level Global with the string bytes,
                     return iconst(Ptr, address) — TODO: globals
                     are Prompt 5.5/5.7 territory; for now stub with
                     null_ptr and a TODO.

Identifier:
  Look up in ctx.local_vars by SymbolId.
  - LocalKind::Ssa(var_id) → builder.use_var(var_id, ir_ty, span)
  - LocalKind::Memory(ptr) → builder.load(ptr, ir_ty, span)
  - Globals/parameters: look up in a separate map (built at function
    entry from Function.params).

Binary arithmetic (+, -, *, /, %, &, |, ^, <<, >>):
  Lower lhs, rhs.
  Apply usual arithmetic conversions (already recorded in implicit_convs;
  the lowered values should already be the common type after applying
  implicit_convs above).
  Emit corresponding opcode (iadd / fadd / etc., with sdiv vs udiv
  chosen by sema's recorded type signedness).

Unary -, ~, !:
  Lower operand.
  - Negation: ineg or fneg
  - Bitwise NOT: bnot
  - Logical NOT: icmp.eq operand, 0 → result is I1; if expr type is int,
    zero-extend to I32

Address-of (&x):
  If x is LocalKind::Memory(ptr), return ptr directly.
  If x is LocalKind::Ssa, this should be impossible — Prompt 5.0 marked
  it address_taken so it would be Memory. Emit an internal-error
  diagnostic if reached.
  Globals: return a constant Ptr to the global symbol.

Dereference (*p):
  Lower p (gives a Ptr Value).
  Emit load with the appropriate result type (sema knows pointee type).

Comparison (<, ==, etc.):
  Lower lhs, rhs.
  icmp / fcmp with appropriate CmpOp / FCmpOp.
  Result is I1 (extend to I32 if expression context expects int).

Logical && / ||:
  SHORT-CIRCUIT via control flow:
    - && : create rhs_block and merge_block. Branch on lhs to either
           rhs_block (if true) or merge_block (with arg=false). In
           rhs_block lower rhs; jump merge_block(rhs_value).
           merge_block has param of type I1; that's the result.
    - || : symmetric, branching to merge_block(true) when lhs is true.

Assignment (=):
  Lower RHS.
  If LHS is a simple identifier:
    - LocalKind::Ssa: builder.def_var(var_id, rhs_value). Result is
      rhs_value.
    - LocalKind::Memory: builder.store(ptr, rhs_value). Result is
      rhs_value (assignment yields the assigned value).
  If LHS is *ptr or struct.field or arr[i]: lower the LHS as an
  address (similar to address-of), then store.

Compound assignment (+=, -=, etc.):
  Equivalent to LHS = LHS op RHS, but evaluate LHS once. Lower LHS to
  get value AND address (if memory) or VarId (if SSA), then perform op
  + reassign.

Pre/post ++/--:
  Like compound assignment with +/- 1. Pre returns the new value, post
  returns the old.

Function call:
  Lower each argument (with their default-argument-promotion conversions
  already in implicit_convs).
  Look up the callee FunctionRef. (May need a callee-name → FunctionRef
  map at module level.)
  Emit call instruction. If function returns void, no result Value.

Cast:
  Lower operand.
  Emit appropriate conversion (sext/zext/trunc/sitofp/etc.) based on
  source and target IrType.
  Pointer↔int casts use inttoptr/ptrtoint.

Sizeof:
  Already evaluated at sema time (ctx.sema.sizeof_kinds for the NodeId).
  - SizeofKind::Constant(n) → iconst(I64, n as i64, span)
  - SizeofKind::RuntimeVla { expr_nodes } → sequence of iadd/imul over
    runtime values. Implement via a helper that walks the recorded
    expressions. For v1, emit a TODO and stub with iconst(0); add an
    integration test that VLAs aren't yet supported, and unstub later.

_Alignof:
  Always constant from sema. Emit iconst.

Ternary (c ? a : b):
  If both arms are pure values without side effects: lower c, a, b;
  emit select.
  Otherwise: branch + block param (similar to short-circuit logic).
  Heuristic: check if a and b are simple expressions (literal, ident,
  member-access). If yes, select. If no, branch.
  Conservative: always use branch in v1; select is a Phase 6 e-graph
  rewrite.

_Generic, comma, compound literal: lower the selected/last/initialized
value and return.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Apply implicit conversions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

After lowering an expression to a Value, look up
ctx.sema.implicit_convs.get(&expr.node_id). If conversions are recorded,
emit them in order:

  ImplicitConversion::IntegerPromotion → sext/zext to I32
  ImplicitConversion::ArithmeticConversion(target_ty) → conversion to target
  ImplicitConversion::IntToFloat → sitofp/uitofp
  ImplicitConversion::FloatToInt → fptosi/fptoui
  ImplicitConversion::FloatConversion → fpext/fptrunc
  ImplicitConversion::PointerToBoolean → icmp.ne ptr, null
  ImplicitConversion::IntegerToBoolean → icmp.ne int, 0
  ImplicitConversion::NullPointerConversion → null_ptr (replace value
    entirely; the int 0 was a placeholder)
  ImplicitConversion::ArrayToPointer → no-op (arrays already lowered as Ptr)
  ImplicitConversion::FunctionToPointer → no-op
  ImplicitConversion::LvalueToRvalue → load from the Memory ptr (already
    handled in identifier lowering for Memory locals, but explicit
    conversions on member-access paths need this)
  ImplicitConversion::QualificationConversion → no-op (qualifiers
    don't exist at IR level)
  ImplicitConversion::BitFieldToInt → load + shift + mask (bit fields
    are Phase 5.5 territory; leave a TODO)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Test files:
  tests/lower_literals.rs
  tests/lower_identifier.rs
  tests/lower_arithmetic.rs
  tests/lower_comparison.rs
  tests/lower_logical.rs       (short-circuit && and ||)
  tests/lower_assignment.rs
  tests/lower_unary.rs
  tests/lower_cast.rs
  tests/lower_call.rs
  tests/lower_ternary.rs

For each, write a small C function (constructed via Phase 4 helpers),
run sema, then lower; assert IR shape (specific opcodes in expected
order).

Example test:

  lower_int_addition_emits_iconst_iconst_iadd_return:
      C: int main() { return 1 + 2; }
      After lowering: entry block has [iconst.i32 1, iconst.i32 2,
      iadd.i32, return]. Assert exact opcode sequence.

  lower_short_circuit_and_creates_three_blocks:
      C: int main(int a, int b) { return a && b; }
      After lowering: entry block branches on a. rhs_block evaluates b.
      merge_block has I1 param, returns it (or zext to i32).
      Assert there are exactly 3 blocks.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
CHECKPOINT
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Standard 5 gates. STOP. Do not commit. Report exit codes, test count,
and any expression kinds that were stubbed with TODO instead of
fully lowered (acceptable list: VLA sizeof, bit-field load/store,
string literals).
```

---

### Prompts 5.4 - 5.7 — Outline only (full text written when we get there)

**Prompt 5.4 — Statement & control flow lowering**
- if/else, while, do-while, for, switch (native opcode), break, continue, goto/label, return
- Block stack for break/continue targets
- Implicit `return` for void functions whose body falls through
- Tests: each control-flow construct in isolation, plus combinations

**Prompt 5.5 — Aggregate lowering (struct, array, GEP)**
- struct/union member access via gep + load
- Array subscript via gep + load
- Bit-field load/store (load + shift + mask + store)
- String literal globals
- Initializer lowering (struct/array brace lists → sequence of stores)
- VLA stack_alloc with runtime size

**Prompt 5.6 — IR text format (printer + parser + round-trip)**
- Printer: produce LLVM-IR-like text format (see §11.5 of phase doc — exact format TBD in prompt)
- Parser: parse text back into Module
- Round-trip test: print → parse → print → assert equal
- Lit tests: input C → emit IR → text-compare to expected

**Prompt 5.7 — Verifier + driver integration + final report**
- Verifier: SSA dominance, type consistency, terminator presence, block param arity, no unreachable blocks
- Verifier runs after every lowering function in debug builds
- `forge --emit-ir file.c` flag
- `forge check` continues to work (sema-only)
- lit tests: end-to-end C → IR for arithmetic, control_flow, functions, structs, globals
- `phases/phase_05_report.md` with completeness matrix, perf, deferrals
- Verdict: READY FOR PHASE 6

---

## 12. Acceptance criteria

- Every C17 expression and statement lowers to IR (with documented exceptions in deferral list)
- IR verifier catches:
    - SSA dominance violations
    - Type mismatches between operand types and opcode expectations
    - Missing or extra terminators per block
    - Block parameter arity mismatches between caller's args and callee block's params
    - **Static alignment violations** — when a constant GEP chain produces a load/store with a constant byte offset, that offset must be a multiple of the access type's alignment. Runtime offsets are exempt; bitcasts and explicit `__attribute__((packed))` flag the access as opt-out (no alignment check).
    - **Volatile/atomic invariants** — verifier rejects e-graph-style "merge two loads of same ptr" if EITHER load is volatile or atomic, even though the verifier is structural and Phase 5 has no e-graph yet (this is a forward guard for Phase 6).
- IR text format round-trips (print → parse → print → equal)
- `forge --emit-ir hello.c` produces correct IR
- 50+ line C function with control flow lowers and verifies clean
- All Phase 4 tests still pass (no regressions)
- `phase_05_report.md` documents completeness and known deferrals

---

## 13. Deferrals to Phase 6+

**Known incomplete IR features:**
- **VLA full lowering** — sizeof handled as iconst placeholder; full runtime VLA in Phase 6 or 7
- **String interning** for literal globals — v1 emits each occurrence separately
- **Trivial-block-param removal** — Braun's optimization for cleaner IR; v1 may emit dead block params
- **Inline assembly** — `__asm__` not handled; flagged at sema, lowered as `unreachable` for now
- **Atomic operations** — `_Atomic` qualifier accepted at sema, lowered as `MemoryOrder::Plain`. Full atomic semantics (`Acquire`/`Release`/etc.) deferred. The MemoryOrder enum has the variants reserved; only the lowering path is incomplete.
- **Floating-point exceptions / rounding mode** — IR doesn't model FE_* state
- **Vector types** — no native IR vector type; vectorization is Phase 7 codegen
- **Setjmp/longjmp** — likely Phase 11 conformance work

**UB exploitation policy (Forge's stance):**

C undefined behavior in arithmetic (signed integer overflow, division by zero, shift overflow, etc.) is **NOT exploited for optimization** in v1.

- Signed overflow: defined as wraparound (two's complement)
- Division/modulo by zero: defined as a runtime trap (or codegen's choice; verifier doesn't reason about it)
- Shift by ≥ width: defined as zero result
- Strict aliasing violations: pointer reinterpretation through `bitcast` is allowed; we do NOT use type-based alias analysis (TBAA) to assume non-aliasing

This makes Forge a **"sane compiler"** — slower than aggressive UB-exploiting compilers (GCC `-O2`, Clang `-O2`) on benchmarks that lean on UB freedoms, but predictable. LLVM-style `nsw`/`nuw`/`exact` flags are deferred until Phase 6 e-graph design forces a decision. Adding them later is a per-opcode optional field, no breaking change.

If a future user wants aggressive UB exploitation as opt-in (`forge -fstrict-overflow`), add it as a Phase 6+ feature.

**Phase 6 e-graph alias representation:**

`restrict` qualifier is captured as a function parameter flag in v3. Per-instruction alias metadata (TBAA-style "this load reads memory of kind X") is deferred. Phase 6 will define how alias information is encoded; that may change the Instruction struct.

---

## 14. Risks

- **Address-taken analysis correctness (Prompt 5.0)** — false negatives mean SSA promotion of memory-resident locals → miscompilation. The escape-rule framing in §3 Decision 1 catches the patterns. Verifier catches some violations (a stack_alloc'd value referenced via def_var) but not all. Test the escape patterns thoroughly in Prompt 5.0.
- **Block param arity bugs in Braun SSA** — easy to mis-thread args through deep loop nests. The verifier's block-param-arity check is the primary safety net.
- **Volatile correctness** — getting `MemoryOrder` wrong is silent miscompilation. Test both:
    - Volatile load/store emitted from `volatile T x` declarations (positive)
    - Plain load/store emitted from non-volatile (negative — verifier should NOT flag plain loads)
- **Alignment check false positives** — packed structs and bitcasts intentionally violate alignment. Verifier must respect `__attribute__((packed))` flag from Phase 4 sema and skip alignment checks on bitcasted pointers.
- **Performance** — IR construction adds work to the pipeline. Phase 4 was 6/28ms (Test A/B); IR may push to 10-15ms. Budget remains comfortable but watch for regressions.
- **Bit-field semantics** — bit-fields are notoriously fiddly in C. Read/write through memory with shift+mask is the standard approach but easy to get wrong on signedness and edge bytes.

---

## 15. Workflow reminders (carry from Phase 4)

- Every prompt ends with explicit exit codes for all 5 gates
- Claude Code does NOT commit — user reviews each report and commits manually
- Test file names are requirements, not suggestions
- Every new feature has at least one test that would fail if removed
- Sanity checks after each prompt (grep for feature keywords, verify production implementations exist)
- Deferred cleanup items accumulate in a list, addressed in 5.7 audit
