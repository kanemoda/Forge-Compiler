# Phase 4 — Semantic Analysis & Type System (Revised v5)

**Depends on:** Phase 3 (Parser) ✅ COMPLETE (766 tests — NodeId added in Prompt 4.0)
**Unlocks:** Phase 5 (IR)
**Estimated duration:** 14–24 days (10 prompts — one preparatory, nine main)

**v5 changelog:** Layered a set of implementation-quality concerns onto v4's correctness work.
- Added explicit borrow-discipline guidance for `SemaContext` (avoid god-object `&mut ctx` patterns; prefer field-level splits; NO `RefCell`).
- Switched to `FxHashMap` / `FxHashSet` from `rustc-hash` for all NodeId-keyed tables; introduced dense `IndexVec`-style storage where appropriate; added `rustc-hash` as a dependency.
- Added a note on qualifier preservation (`restrict`, `_Atomic`, `volatile` are downstream contracts, not Phase 4 flags alone — preserve faithfully through type manipulations).
- Clarified that tentative-definition resolution is Sema's bookkeeping; the `.comm` vs `.bss` decision is Codegen's (Phase 7), not Phase 4's.
- Documented macro-expansion backtrace diagnostics as a known Phase 4 limitation (Phase 2 work is required and will land separately).
- Sterilized all commit-related instructions across the doc. Claude Code runs the gate commands and reports; the user commits.

**v4 retained items:** enum underlying type widening, array parameter qualifier transfer, `_Noreturn`/`inline` function specifiers, FAM structural restrictions, VLA `sizeof` runtime evaluation, function pointer dereference cycle note, parameter/outermost-block same-scope clarification.

---

## Goal

Transform the raw AST from Phase 3 into a **Typed AST (TAST)** where every expression has a resolved type, every name is resolved to a symbol, and every ill-formed construct produces a clear diagnostic. After this phase, the TAST is ready for IR lowering (Phase 5).

---

## Key Design Decisions

### 1. TAST architecture: annotated AST with NodeId side tables

Two approaches exist: (a) create a completely new tree, (b) annotate the existing AST in-place.

**We use approach (b) with side tables keyed by NodeId.** The existing AST nodes stay as-is. A `TypedAst` struct holds:
- `expr_types: HashMap<NodeId, QualType>` — resolved type of every expression
- `implicit_conversions: HashMap<NodeId, Vec<ImplicitConversion>>` — conversions inserted before an expression
- `symbol_refs: HashMap<NodeId, SymbolId>` — which symbol an identifier refers to
- `lvalue_map: HashSet<NodeId>` — which expressions are lvalues

This avoids duplicating the entire AST tree while still annotating every node.

**NodeId source — a preparatory step is required:**
Phase 3's AST does NOT currently emit `NodeId`s — the parser was built with `Span`-only identification. Before any Phase 4 work begins, **Prompt 4.0 (below) adds `NodeId` to the parser**. After that prep, every `Expr`, `Stmt`, and `Decl` carries a sequential `u32` ID assigned during parsing. Phase 4 uses these IDs as the sole key for all side tables.

**Do NOT use `Span` as a key.** Macro expansion can produce multiple expressions sharing a span, which silently corrupts the map. If an AST node is found without a `NodeId` after Prompt 4.0, that is a bug in 4.0 — file it and fix there, not by falling back to `Span`.

**Alternative considered:** A fully separate TAST tree. This would be cleaner for Phase 5 but doubles the type definitions. We can migrate later if needed.

### 2. Implicit conversions are explicit nodes

C has ~13 kinds of implicit conversion. Instead of silently changing types, we record each conversion explicitly:

```rust
pub enum ImplicitConversion {
    LvalueToRvalue,          // load from memory
    ArrayToPointer,          // int[10] → int*
    FunctionToPointer,       // void(int) → void(*)(int)
    IntegerPromotion { to: Type },
    ArithmeticConversion { to: Type },
    IntToFloat { to: Type },
    FloatToInt { to: Type },
    FloatConversion { to: Type },
    PointerToBoolean,        // any pointer → _Bool
    NullPointerConversion,   // literal 0 or (void*)0 → T*
    IntegerToPointer,        // (T*)n  (implementation-defined)
    PointerToInteger,        // (n_t)p (implementation-defined)
    QualificationConversion, // int* → const int*
    BitFieldToInt,           // bit-field → int for value ops
}
```

This makes IR lowering straightforward — each conversion maps to a specific IR instruction.

### 3. Type specifier resolution: complete combinatorial table

C allows type specifiers in any order. `long unsigned int long` = `unsigned long long`. The resolver must handle all valid combinations (full table in Prompt 4.2).

### 4. String literals are arrays, not pointers

`"hello"` has type `char[6]` (5 chars + null terminator). It **decays** to `char*` in most expression contexts via `ArrayToPointer` implicit conversion. This distinction matters for `sizeof("hello")` → 6, not 8.

### 5. Integer literal type depends on value AND suffix

C17 §6.4.4.1: an unsuffixed decimal integer literal has the first type in which its value fits: `int`, `long`, `long long`. Octal/hex also consider unsigned variants. Value-dependent — `2147483647` is `int`, `2147483648` is `long` (LP64).

### 6. Struct layout with padding and bit-fields

Struct layout must match the x86-64 System V ABI: each member aligned to its requirement, padding inserted, struct total size padded to its alignment. Bit-fields follow GCC behavior.

### 7. Error recovery strategy — accumulate, never abort

**All analysis functions have the signature pattern `fn analyze_X(..., ctx: &mut SemaContext) -> Option<TypedX>`.** Errors are pushed into `ctx.diagnostics: Vec<Diagnostic>` and `None` is returned for the broken subtree. `Result<T, Diagnostic>` is NOT used in analysis APIs — only in pure helpers like `resolve_type_specifiers` that have no `&mut ctx` available.

Rationale: a type error in one function must not abort analysis of other functions. The driver reports all accumulated diagnostics at the end.

### 8. Four C namespaces — enumerated explicitly

C17 §6.2.3 defines four disjoint name namespaces. We model all four:

1. **Label namespace** — goto targets, function-scoped. Lives on `FnContext`.
2. **Tag namespace** — `struct`/`union`/`enum` tags. `Scope.tags`.
3. **Member namespace** — each struct/union has its own member map. Lives inside `StructLayout` / `UnionLayout`, not in `Scope`.
4. **Ordinary namespace** — everything else (variables, functions, typedefs, enum constants, parameters). `Scope.symbols`.

These are independent: `struct x { int x; } x;` is legal — three different `x`s in three different namespaces.

### 9. Type compatibility is its own function

C uses "compatible types" as a distinct relation from "identical types" in ~15 places. We implement it once as `are_compatible(t1, t2, ctx) -> bool` and call it everywhere. Details in Prompt 4.1 Section 7.

### 10. Constant expression evaluator is built BEFORE declaration analysis

The const evaluator is needed for enum values, array sizes, bit-field widths, and `_Static_assert` — all of which appear in declarations. It is therefore built in Prompt 4.2.5, BEFORE declaration analysis (Prompt 4.3). This avoids any forward-reference or stub pattern.

### 11. Two flavors of constant expression

C17 distinguishes:
- **Integer constant expression** (§6.6p6): case labels, enum values, bit-field widths, `_Static_assert`, array sizes in non-VLA arrays.
- **Constant expression** (general): static-storage initializers, which additionally allow **address constants** like `&x` or `&arr[3]`.

We implement `eval_icx()` for the first and defer full address-constant support to Phase 5 (note it in the code with a TODO). `eval_icx()` is sufficient for every Phase 4 use site.

---

## Deliverables

1. **`forge_sema` crate** — semantic analysis pass
2. **Type system** — `Type`, `QualType`, utility methods, `TargetInfo`, `are_compatible`, `Display for QualType`
3. **Type specifier resolver** — combinatorial table for all valid C17 combinations
4. **Symbol table** — scoped name resolution with tag/label/ordinary namespaces
5. **Integer constant expression evaluator** — for array sizes, case labels, enum values, static assertions, bit-field widths
6. **Expression type checker** — type for every expression, all operators validated, `_Alignof` included
7. **Implicit conversion insertion** — explicit conversion records in the TAST
8. **Lvalue/rvalue analysis** — which expressions are assignable
9. **Struct/union layout** — field offsets, padding, alignment, bit-fields, self-referential and mutually recursive tags, anonymous members (C11), `_Alignas` respected
10. **Statement validation** — control flow checks, return type, break/continue context
11. **Comprehensive tests** — unit, lit, real-program analysis

---

## Claude Code Prompts

> **CHECKPOINT RULE (applies to every prompt below):**
> At the end of each prompt's work, STOP and run:
> ```
> cargo build -p forge_sema
> cargo test -p forge_sema
> cargo clippy -p forge_sema --all-targets -- -D warnings
> cargo fmt -p forge_sema -- --check
> ```
> All four must pass. Then **STOP** — do not commit. Report a one-line summary
> of what changed and the test count. The user commits manually. Do not
> layer new work on a broken or untested foundation.

### Prompt 4.0 — Preparatory: Add NodeId to the parser AST

```
This is a preparatory step for Phase 4. We do NOT touch forge_sema yet.
Phase 4 needs a stable, unique identifier on every AST node that can be
referenced from the semantic side tables (expression types, implicit
conversions, symbol references, lvalue flags). Span is not usable as
a key because macro expansion can produce multiple nodes sharing a span.

This prompt adds NodeId to forge_parser and threads it through construction.
It must land as a standalone change before any work on forge_sema begins.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — NodeId type
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

In forge_parser/src/ast.rs (or a new forge_parser/src/node_id.rs):

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct NodeId(pub u32);

impl NodeId {
    pub const DUMMY: NodeId = NodeId(u32::MAX);   // for tests that build ASTs by hand
}

Derive Default if needed. Export from lib.rs.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — AST node coverage
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Add a `node_id: NodeId` field (alongside the existing `span: Span`) to
every variant that the semantic analyzer will annotate. At minimum:

  - Every variant of `Expr`
  - Every variant of `Stmt`
  - Every variant of `Declaration` / `InitDeclarator`
  - `FunctionDef`
  - `TypeName` (used in sizeof, cast, _Generic, etc.)
  - `Initializer`
  - `StructField` (so bit-field widths and designators can be referenced)

If these are enums with structs inside (e.g. `Expr::BinaryOp { ... }`),
add `node_id` to each struct. Do NOT try to refactor to a single wrapper
struct holding `(NodeId, Span, ExprKind)` — too invasive for a prep step;
a per-variant field is fine.

Do NOT add NodeId to:
  - Token / TokenKind (lexer-level, not AST)
  - Type specifiers (TypeSpecifierToken) — those are sub-parts, not nodes
  - Declarator / DirectDeclarator — these are sub-parts of declarations

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — ID generation
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

In the Parser struct:
    next_node_id: u32,

In Parser::new or the equivalent constructor, initialize to 0.

Add a method:
    fn next_id(&mut self) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        NodeId(id)
    }

At EVERY construction site of an AST node variant listed in Section 2,
obtain a fresh NodeId from `self.next_id()` and store it in the new field.
Construction sites are spread across expr.rs, stmt.rs, decl.rs — walk each
file and add the call.

Invariant: across a single parse of a single translation unit, each NodeId
is unique. IDs are NOT stable across parses (don't persist them to disk).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Macro-expanded nodes
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

If the preprocessor produces tokens with identical spans (macro expansion),
the parser still assigns each resulting AST node its own unique NodeId via
next_id(). This is precisely why Span cannot be used as a key and NodeId
can: two expressions from the same macro call have the same Span but
different NodeIds.

No behavioural change needed in the preprocessor for this — the parser
produces distinct AST nodes regardless of shared spans.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Update tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Some existing parser tests construct AST nodes by hand (not via the parser).
Those tests break because the new field is missing. Two options:

(a) Give hand-built nodes NodeId::DUMMY. This is the correct answer for
    tests that never go through sema.
(b) Give them a unique counter if the test inspects multiple nodes.

Update tests/helpers if needed to provide a small helper:
    fn dummy_node_id() -> NodeId { NodeId::DUMMY }

All existing 764 parser tests must still pass after this prompt.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 6 — Sanity test
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Add a single new test in forge_parser:
  - Parse `int main(void) { int x = 1 + 2; return x; }`
  - Walk the resulting AST
  - Collect every NodeId
  - Assert: no duplicates; count matches expected node count
  - Assert: IDs are sequential from 0 to N-1 (or at least dense)

Run the smoke test:
  - Parse stdio.h again (benchmark from Phase 3 — 14ms target still holds
    approximately; NodeId adds a u32 per node, negligible)
  - No regression in parse time > 20% (generous budget; should be ~0%)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
CHECKPOINT
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

cargo build
cargo test -p forge_parser                             → all pass
cargo test --all                                       → all pass (764+1 tests)
cargo clippy -p forge_parser --all-targets -- -D warnings
cargo fmt -p forge_parser -- --check

STOP. Do not commit. Report the test count and a one-line summary of
changes. The user will review and commit.
```

### Prompt 4.1 — Type system, QualType, TargetInfo, compatibility, Display

> **STATUS:** Dispatched before v5. The prompt body is preserved below for
> reference. Two v5 concerns apply retroactively when you reach them in
> later prompts:
> 1. **Hashing:** any `HashMap`/`HashSet` keyed by `NodeId` or small integer
>    types should use `rustc_hash::FxHashMap` / `FxHashSet`. Add
>    `rustc-hash = "2"` to `forge_sema/Cargo.toml` the first time you need it
>    (likely in Prompt 4.3 when `SemaContext` materializes).
> 2. **Qualifier preservation:** `restrict`, `_Atomic`, and `volatile` on
>    `QualType` are not just flags for Phase 4 — they are downstream
>    contracts. `restrict` drives alias analysis in Phase 5/6. `_Atomic`
>    drives memory-ordering lowering. `volatile` forbids load/store
>    elision. Do NOT silently drop these qualifiers when performing type
>    manipulations (compositing, adjusting array parameters, etc.). A test
>    in Prompt 4.8 will verify round-trip preservation.

```
Create the forge_sema crate in the Forge workspace. Implement the core type system.

This is the foundation everything else builds on — get the types right and
everything downstream is cleaner.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Type enum
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create forge_sema/src/types.rs:

pub enum Type {
    Void,
    Bool,                                      // _Bool
    Char { signedness: Signedness },           // plain, signed, unsigned
    Short { is_unsigned: bool },
    Int { is_unsigned: bool },
    Long { is_unsigned: bool },
    LongLong { is_unsigned: bool },
    Float,
    Double,
    LongDouble,
    Pointer { pointee: Box<QualType> },
    Array { element: Box<QualType>, size: ArraySize },
    Function {
        return_type: Box<QualType>,
        params: Vec<ParamType>,
        is_variadic: bool,
        is_prototype: bool,   // false for old-style `int f()` with no prototype
    },
    Struct(StructTypeId),
    Union(UnionTypeId),
    Enum(EnumTypeId),
}

pub enum Signedness { Plain, Signed, Unsigned }

pub struct ParamType {
    pub name: Option<String>,
    pub ty: QualType,
}

pub enum ArraySize {
    Fixed(u64),          // int arr[10]
    Variable,            // VLA: int arr[n]
    Incomplete,          // int arr[] (extern or flexible member)
    Star,                // int arr[*] (prototype-scope VLA)
}

pub struct QualType {
    pub ty: Type,
    pub is_const: bool,
    pub is_volatile: bool,
    pub is_restrict: bool,
    pub is_atomic: bool,
    pub explicit_align: Option<u64>,   // from _Alignas / __attribute__((aligned(N)))
}

impl QualType:
  - unqualified(ty: Type) -> QualType
  - with_const(self) -> QualType
  - strip_qualifiers(&self) -> Type     // value, not reference (unqualified clone)
  - has_any_qualifier(&self) -> bool

pub struct StructTypeId(pub u32);
pub struct UnionTypeId(pub u32);
pub struct EnumTypeId(pub u32);

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Type utility methods
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Predicates on Type:
  is_void, is_bool, is_integer, is_floating, is_arithmetic, is_scalar,
  is_pointer, is_array, is_function, is_struct, is_union, is_struct_or_union,
  is_complete(&self, ctx: &TypeContext) -> bool
  is_unsigned(&self) -> bool  // for integer types
  is_null_pointer_constant(expr_type: &Type, const_value: Option<i64>) -> bool

Integer rank (C17 §6.3.1.1):
  integer_rank(&self) -> u8
    Bool=0, Char=1, Short=2, Int=3, Long=4, LongLong=5
    Enum = rank of Int

Size/alignment:
  size_of(&self, target: &TargetInfo, ctx: &TypeContext) -> Option<u64>
  align_of(&self, target: &TargetInfo, ctx: &TypeContext) -> Option<u64>
    (None for incomplete, void, VLA)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — TargetInfo
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct TargetInfo {
    pub pointer_size: u64,
    pub pointer_align: u64,
    pub char_is_signed: bool,
    pub bool_size: u64,
    pub char_size: u64,
    pub short_size: u64,
    pub int_size: u64,
    pub long_size: u64,
    pub long_long_size: u64,
    pub float_size: u64,
    pub double_size: u64,
    pub long_double_size: u64,
    pub long_double_align: u64,
    pub max_align: u64,
}

impl TargetInfo:
  pub fn x86_64_linux() -> TargetInfo
  pub fn size_t_type(&self) -> Type
  pub fn ptrdiff_t_type(&self) -> Type
  pub fn wchar_t_type(&self) -> Type

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Integer promotions and usual arithmetic conversions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub fn integer_promotion(ty: &Type, target: &TargetInfo) -> Type
  C17 §6.3.1.1: rank < int promotes to int.
  Bool/Char/Short/Enum → Int(signed).
  Unsigned short on LP64 fits in signed int → Int(signed).
  Int and above unchanged.

pub fn usual_arithmetic_conversions(lhs: &Type, rhs: &Type, target: &TargetInfo) -> Type
  C17 §6.3.1.8:
  1. If either is long double → long double
  2. If either is double → double
  3. If either is float → float
  4. Apply integer promotions to both
  5. If same type → that type
  6. Both signed or both unsigned → higher rank wins
  7. Unsigned rank ≥ signed rank → unsigned type
  8. Signed can represent all unsigned values → signed type
  9. Otherwise → unsigned version of signed type

  THE most important function in the type system. Every binary operator calls it.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — ImplicitConversion enum
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub enum ImplicitConversion {
    LvalueToRvalue,
    ArrayToPointer,
    FunctionToPointer,
    IntegerPromotion { to: Type },
    ArithmeticConversion { to: Type },
    IntToFloat { to: Type },
    FloatToInt { to: Type },
    FloatConversion { to: Type },
    PointerToBoolean,
    NullPointerConversion,
    IntegerToPointer,
    PointerToInteger,
    QualificationConversion,
    BitFieldToInt,
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 6 — Display for QualType (C-syntax pretty printer)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

REQUIRED for diagnostics. Error messages like
  "incompatible types: expected 'int *', found 'char'"
need this.

impl Display for QualType:
  Render the type in C syntax. Qualifiers before the base type where natural:
    const int            not    Int { is_const: true }
    const int *          pointer to const int
    int *const           const pointer to int
    int (*)(int, int)    pointer to function taking (int,int) returning int
    int [10]             array of 10 int
    struct Foo           tagged struct (use tag name; "struct <anonymous>" if no tag)

Implementation approach: C declarator syntax is recursive inside-out, but for
error messages a simpler left-to-right form is acceptable. Use an "abstract
declarator" style: base specifier on the left, then pointers/arrays/functions
in declaration order. When rendering a function pointer specifically, emit
the standard `ReturnType (*)(ParamTypes)` form because users will recognize it.

Provide a helper: QualType::to_c_string(&self, ctx: &TypeContext) -> String
that handles struct/union/enum name lookup through ctx. The Display impl
formats without struct-name lookup (falls back to "struct #<id>") — for
user-facing diagnostics, prefer to_c_string.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 7 — Type compatibility
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

C uses "compatible types" (C17 §6.2.7) as a distinct relation from identity.
It appears in ~15 places: multiple extern declarations, function prototypes,
pointer comparisons, _Generic selection, assignment, etc.

pub fn are_compatible(a: &QualType, b: &QualType, ctx: &TypeContext) -> bool

Rules:
1. Same qualifiers required for the outermost type in most uses — BUT some
   callers need unqualified compatibility. Provide two:
     are_compatible(a, b, ctx)            — requires matching qualifiers
     are_compatible_unqualified(a, b, ctx) — compares ty only

2. Types are compatible if:
   - Same arithmetic type
   - Both pointers AND pointees are compatible
   - Both arrays, element types compatible, sizes match OR one is incomplete
   - Both functions: return types compatible, params compatible one-for-one,
     variadic-ness matches. Special rule: unprototyped `()` is compatible
     with any prototyped form whose params match default-promoted types.
   - Same struct/union tag (C: struct types are compatible only with themselves,
     per translation unit — two unnamed structs with identical members are NOT
     compatible)
   - Same enum OR enum vs its underlying integer type

pub fn composite_type(a: &QualType, b: &QualType, ctx: &TypeContext) -> QualType
  Required by C17 §6.2.7 for merging multiple declarations.
  - For arrays: if one has size and the other is incomplete, composite has the size
  - For functions: if one is prototyped and the other isn't, use the prototyped
  - Otherwise returns a (caller-ensured compatible with b)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 8 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Type sizes (x86-64 LP64):
  sizeof(char)=1, short=2, int=4, long=8, long long=8, ptr=8, _Bool=1,
  float=4, double=8, void=None

Integer promotion:
  char→int, unsigned char→int, short→int, unsigned short→int (LP64),
  int→int, long→long, _Bool→int

Usual arithmetic conversions:
  int+int→int, int+long→long, int+unsigned int→unsigned int,
  long+unsigned int→long (LP64), int+double→double, float+double→double,
  unsigned long+signed long→unsigned long,
  char+char→int (both promote first!),
  int+unsigned long long→unsigned long long

QualType:
  unqualified(Int) has no qualifiers; with_const adds const; strip_qualifiers clears

Display:
  "int", "const int", "int *", "const int *", "int *const",
  "int [10]", "int (*)(int, int)", "struct Foo", "unsigned long long"

Compatibility:
  int ~ int, const int !~ int (qualified), int* ~ int* (unqualified),
  int[10] ~ int[10], int[10] ~ int[] (one incomplete), int[10] !~ int[20],
  int(int) ~ int(int), int(int) ~ int() (prototype vs unprototyped, compatible),
  struct A {int x;} !~ struct B {int x;} (different tags)

Composite type:
  int[] + int[10] → int[10]; int() + int(int) → int(int)

Crate setup:
  Add forge_sema to workspace Cargo.toml.
  Dependencies: forge_parser, forge_lexer, forge_diagnostics.
```

### Prompt 4.2 — Symbol table + type specifier resolver

```
Implement the symbol table and type specifier resolution.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Symbol table with four namespaces
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create forge_sema/src/scope.rs:

pub type SymbolId = u32;
pub type TagId = u32;

pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub ty: QualType,
    pub kind: SymbolKind,
    pub storage: StorageClass,
    pub linkage: Linkage,
    pub span: Span,
    pub is_defined: bool,
}

pub enum SymbolKind {
    Variable,
    Function,
    Typedef,
    EnumConstant { value: i64, enum_id: EnumTypeId },
    Parameter,
}

pub enum StorageClass { None, Auto, Register, Static, Extern, ThreadLocal }
pub enum Linkage { None, Internal, External }
pub enum ScopeKind { File, Function, Block, Prototype }

pub struct Scope {
    pub kind: ScopeKind,
    pub symbols: HashMap<String, SymbolId>,   // ordinary namespace
    pub tags: HashMap<String, TagId>,          // tag namespace
}

pub enum TagEntry {
    Struct(StructTypeId),
    Union(UnionTypeId),
    Enum(EnumTypeId),
    // Entries can be incomplete (StructLayout/UnionLayout holds a `is_complete` flag)
}

pub struct SymbolTable {
    scopes: Vec<Scope>,
    all_symbols: Vec<Symbol>,
    all_tags: Vec<TagEntry>,
}

NAMESPACE NOTES — C17 §6.2.3 defines 4 namespaces:
  1. Labels         → on FnContext (added in Prompt 4.6), NOT here
  2. Tags           → Scope.tags (struct/union/enum names)
  3. Members        → inside StructLayout/UnionLayout, NOT here
  4. Ordinary       → Scope.symbols (variables, functions, typedefs,
                                      enum constants, parameters)
  Members are per-struct, not scoped. Labels are per-function. We handle
  them in their owning structures, not in Scope.

Methods:
  push_scope(kind), pop_scope(), current_scope() -> &Scope, current_scope_kind()

  // Ordinary namespace
  declare(&mut self, symbol: Symbol, ctx: &mut SemaContext) -> Option<SymbolId>
    Duplicate in same scope → push error to ctx.diagnostics, return None.
    EXCEPTION: compatible redeclarations (extern, tentative defs) — merge.
    Use are_compatible() from Prompt 4.1. Merged entry becomes the composite type.
  lookup(&self, name: &str) -> Option<&Symbol>
  lookup_in_current_scope(&self, name: &str) -> Option<&Symbol>

  // Tag namespace
  declare_tag(&mut self, name: &str, entry: TagEntry, ctx: &mut SemaContext)
      -> Option<TagId>
    Incomplete-then-complete is allowed (struct forward decl then full def).
    Redeclaring with different kind (struct vs union) → error.
  lookup_tag(&self, name: &str) -> Option<(TagId, &TagEntry)>
  complete_tag(&mut self, id: TagId, layout: ...)   // upgrade incomplete → complete

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Type specifier resolver
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create forge_sema/src/resolve.rs:

━━━ EXTEND ParamType FROM PROMPT 4.1 ━━━
Prompt 4.1 defined ParamType { name, ty }. Add a new field here:
    pub struct ParamType {
        pub name: Option<String>,
        pub ty: QualType,
        pub has_static_size: bool,   // NEW: from `T[static N]` param syntax
    }
Default has_static_size = false for all existing construction sites.
This is a backward-compatible additive change.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub fn resolve_type_specifiers(
    specifiers: &DeclSpecifiers,
    table: &SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<QualType>

Algorithm:
1. Count each keyword: signed, unsigned, short, long, int, char, void,
   float, double, bool, complex.
2. If a TypedefName is present, no other type keywords allowed (qualifiers OK).
   Look it up. Apply extra qualifiers on top.
3. If a Struct/Union/Enum specifier is present, look up or define the tag.
4. Otherwise resolve the combination by table:

   void                                          → Void
   _Bool                                         → Bool
   [signed|unsigned]? char                        → Char(signedness)
   [signed|unsigned]? short [int]?                → Short
   [signed|unsigned]? [int] alone                  → Int
   signed                                         → Int(signed)
   unsigned                                       → Int(unsigned)
   [signed|unsigned]? long [int]?                 → Long
   [signed|unsigned]? long long [int]?            → LongLong
   float                                         → Float
   double                                        → Double
   long double                                   → LongDouble
   float _Complex / double _Complex / long double _Complex → Complex variants

   Specifiers in ANY order: "long unsigned int long" == "unsigned long long".

5. Apply type qualifiers (const, volatile, restrict, atomic) from specifiers.
6. Apply _Alignas(N) or __attribute__((aligned(N))) to explicit_align.

Errors (push to ctx.diagnostics, return None):
  float double, unsigned float, short long, void signed,
  three longs, multiple storage classes, multiple typedef names,
  typedef name + type keyword.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Declarator to type
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub fn resolve_declarator(
    base_type: QualType,
    declarator: &Declarator,
    is_parameter: bool,
    table: &SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<(Option<String>, QualType)>

Walk outside-in:
  - Apply pointer prefixes, carrying pointer qualifiers.
  - Recurse into Parenthesized(inner).
  - Array suffix → Array type. If is_parameter, adjust `T[N]` → `T*` (C17 §6.7.6.3).
    Array size expression is evaluated via eval_icx (see Prompt 4.2.5).

    ARRAY PARAMETER QUALIFIER TRANSFER (C99/C17 §6.7.6.3p7):
    C allows qualifiers and `static` INSIDE the brackets of an array parameter:
      void f(int arr[const 10]);     // → int *const arr
      void f(char s[restrict]);      // → char *restrict s
      void f(int arr[static 10]);    // → int *arr, plus "at least 10" hint
      void f(int arr[const static 10]);  // → int *const arr, + hint

    When is_parameter and we adjust T[N] → T*:
      - Transfer const/volatile/restrict/atomic from the array brackets onto
        the RESULTING POINTER (not the pointee). `int arr[const]` becomes
        `int *const arr`, NOT `const int *arr`.
      - `static` is a hint ("caller guarantees ≥N elements"); record it on
        the parameter (ParamType.has_static_size: bool) but do not error.
        Phase 5 may use it for optimization; Phase 4 just preserves it.
      - A VLA-in-parameter `int arr[n]` also adjusts to `int *`; the size
        expression is type-checked but discarded.
  - Function suffix → Function type. Push Prototype scope while resolving params.
    An empty parameter list in a declarator `int f();` is NOT a prototype —
    mark Function.is_prototype = false. `int f(void)` IS a prototype with 0 params.
  - Identifier(name) → captures the declared name.

Inside-out effective type: `int (*fp)(int)` → pointer to function(int)→int.

Helper: pub fn declarator_name(decl: &Declarator) -> Option<String>

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — _Alignas handling
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

_Alignas(N) where N is an integer constant expression:
  Evaluate N via eval_icx (stubbed for now, wired in 4.2.5).
  Must be a power of 2, >= natural alignment of the type.
  Set QualType.explicit_align = Some(N).

_Alignas(type-name):
  Equivalent to _Alignas(_Alignof(type-name)).

Error if _Alignas weakens alignment (smaller than natural).
Error if value is not a power of 2.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Symbol table:
  Declare in file scope, lookup succeeds.
  Declare in inner scope shadows outer. Pop scope restores outer.
  Duplicate in same scope → error (diagnostics pushed).
  Two extern with compatible types → OK (merged).
  Tag namespace independent: `struct foo { int x; }; int foo;` → both valid.
  Struct forward decl then full def → tag completed, not duplicated.

Specifier resolution (✓ = expect success):
  ✓ int → Int(signed)
  ✓ unsigned int → Int(unsigned)
  ✓ unsigned → Int(unsigned)       (bare unsigned)
  ✓ signed → Int(signed)           (bare signed)
  ✓ long int → Long(signed)
  ✓ long → Long(signed)            (int optional)
  ✓ long long → LongLong(signed)
  ✓ unsigned long long int → LongLong(unsigned)
  ✓ long unsigned int long → LongLong(unsigned)  (any order)
  ✓ short, signed char, unsigned char, char
  ✓ float, double, long double, _Bool
  ✓ const int → Int with is_const
  ✗ float double, unsigned float, short long, long long long

Declarator:
  int x, int *p, int **p, const int *p, int *const p,
  int arr[10], int (*fp)(int, int),
  void (*signal(int, void(*)(int)))(int)

Array parameter qualifier transfer (C99/C17):
  void f(int arr[const 10]) → f's parameter type is `int *const`
  void f(char s[restrict]) → param type is `char *restrict`
  void f(int arr[static 10]) → param type `int *`, has_static_size=true
  void f(int arr[const static 10]) → `int *const`, has_static_size=true
  NOT a parameter: `int arr[const 10];` as a variable → error in C (const is
    not allowed in array brackets outside function parameters)

_Alignas:
  _Alignas(16) char buf[64]; → explicit_align = Some(16)
  _Alignas(double) char buf[64]; → explicit_align = Some(align_of(double)) = 8
  _Alignas(1) int x; → error (weakens)
  _Alignas(3) int x; → error (not power of 2)
```

### Prompt 4.2.5 — Integer constant expression evaluator

```
Implement the integer constant expression evaluator BEFORE declaration analysis
uses it. This is a small, focused pass.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — ConstValue and eval_icx
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create forge_sema/src/const_eval.rs:

pub enum ConstValue {
    Integer(i64),
    Unsigned(u64),
    Float(f64),
}

impl ConstValue:
  to_i64(&self) -> Option<i64>   // for case labels, array sizes, enum values
  to_u64(&self) -> Option<u64>
  is_zero(&self) -> bool

pub fn eval_icx(
    expr: &Expr,
    ctx: &mut SemaContext,
) -> Option<ConstValue>

Supported constructs (per C17 §6.6):
  Integer literals, character literals, enum constants (look up in symbol table),
  sizeof(type) and sizeof(expr) (non-evaluating),
  _Alignof(type),
  __alignof__(type) and __alignof__(expr)   (GCC extensions, same rule as sizeof),
  Unary: +, -, ~, !
  Binary: + - * / % << >> & | ^ && || == != < > <= >=
  Ternary: c ? a : b
  Cast: (int-type)expr   — only to integer types within eval_icx
  Parenthesized

Rejected (push diagnostic, return None):
  Variable references (non-enum-constant)
  Function calls
  Assignment / compound assignment
  Comma
  Pointer arithmetic
  Increment/decrement
  Address-of, dereference
  Compound literal

Division/modulo by zero → push diagnostic, return Some(ConstValue::Integer(0))
to allow analysis to continue without cascading errors.

Signed overflow: wrap silently using i64::wrapping_*, matching the preprocessor's
#if evaluator behavior from Phase 2. Unsigned operations use u64 wrapping.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Helpers for caller sites
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub fn eval_icx_as_i64(expr, ctx) -> Option<i64>
  Calls eval_icx, then to_i64. For case labels, enum values, array sizes.

pub fn eval_icx_as_u64(expr, ctx) -> Option<u64>
  For sizes and bit-field widths (non-negative).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Note on general constant expressions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

C17 also defines "constant expressions" (general form, §6.6p9) which allow
address constants like `&x` and `&arr[3]` for static-storage initializers.
We do NOT implement those in Phase 4 — leave a TODO in const_eval.rs and
in analyze_declaration's static initializer path. Phase 5 IR lowering handles
address constants via a different path.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

2 + 3 → 5
1 << 10 → 1024
(int)3.14 → 3
sizeof(int) → 4
sizeof(int[10]) → 40
_Alignof(double) → 8
1 / 0 → diagnostic + 0
x (variable) → diagnostic (not a constant expression)
f() → diagnostic
enum { A = 5 }; then A+1 → 6
(1 ? 7 : 8) + 1 → 8

Re-wire _Alignas(N) in Prompt 4.2 to call eval_icx_as_u64 now that it exists.
```

### Prompt 4.3 — Declaration analysis + struct layout

```
Implement declaration analysis and struct/union layout computation.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — SemaContext (with storage strategy and borrow discipline)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

DEPENDENCIES:
  Add to forge_sema/Cargo.toml:
    rustc-hash = "2"          # FxHashMap / FxHashSet — u32-friendly hasher
  (This replaces std HashMap for NodeId-keyed tables. Do NOT use std HashMap
  for any integer-keyed table in forge_sema.)

use rustc_hash::{FxHashMap, FxHashSet};

pub struct SemaContext {
    pub table: SymbolTable,
    pub target: TargetInfo,
    pub diagnostics: Vec<Diagnostic>,

    pub struct_defs: Vec<StructLayout>,
    pub union_defs: Vec<UnionLayout>,
    pub enum_defs: Vec<EnumLayout>,

    // Expression annotations. Storage strategy — table by table:
    //   expr_types:    DENSE. Every expression gets a type. Use IndexVec-style
    //                  storage: Vec<Option<QualType>> indexed by NodeId.0 as
    //                  usize. Resize when a new NodeId exceeds current len.
    //   implicit_convs: SPARSE. Most expressions have no conversion; only ~20%
    //                  do. FxHashMap.
    //   symbol_refs:   SPARSE. Only identifier expressions. FxHashMap.
    //   lvalues:       DENSE-ISH. Use FxHashSet<NodeId> for simplicity; if
    //                  profiling later shows it matters, switch to a packed
    //                  bitvec indexed by NodeId.
    //   sizeof_kinds:  VERY SPARSE. FxHashMap.
    //
    // Rationale: std HashMap uses SipHash, which is ~3x slower than FxHash for
    // u32 keys and unnecessary for compiler-internal use (no HashDoS threat).
    // rustc, Clang/Swift, and Cranelift all use a Fx-style hasher for this.

    pub expr_types: Vec<Option<QualType>>,            // dense, NodeId.0 indexed
    pub implicit_convs: FxHashMap<NodeId, Vec<ImplicitConversion>>,
    pub symbol_refs: FxHashMap<NodeId, SymbolId>,
    pub lvalues: FxHashSet<NodeId>,

    // Populated by check_sizeof in Prompt 4.4; consumed by Phase 5.
    pub sizeof_kinds: FxHashMap<NodeId, SizeofKind>,
}

// SizeofKind is defined in Prompt 4.4; forward-declare in types.rs as:
//   pub enum SizeofKind { Constant(u64), RuntimeVla { expr_nodes: Vec<NodeId> } }
// and re-export from forge_sema::lib.

impl SemaContext {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.is_error())
    }
    pub fn error(&mut self, span: Span, msg: impl Into<String>);
    pub fn warn(&mut self, span: Span, msg: impl Into<String>);

    /// Record the type of an expression. Grows expr_types if NodeId exceeds
    /// current length.
    pub fn set_type(&mut self, id: NodeId, ty: QualType) {
        let idx = id.0 as usize;
        if idx >= self.expr_types.len() {
            self.expr_types.resize(idx + 1, None);
        }
        self.expr_types[idx] = Some(ty);
    }

    pub fn get_type(&self, id: NodeId) -> Option<&QualType> {
        self.expr_types.get(id.0 as usize).and_then(|o| o.as_ref())
    }
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1.5 — Borrow discipline (READ BEFORE WRITING ANY ANALYSIS FUNCTION)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

SemaContext has many fields. If you write `fn foo(ctx: &mut SemaContext)` and
then inside foo try to hold `&ctx.table` and `&mut ctx.diagnostics` at the
same time, the borrow checker will reject it — and it's RIGHT to: the whole
point of &mut is exclusive access.

The three patterns to use, in order of preference:

PATTERN A — Field-level split borrows (PREFERRED for hot paths):
  Functions take explicit sub-references:
      fn check_ident(
          name: &str,
          span: Span,
          id: NodeId,
          table: &SymbolTable,
          expr_types: &mut Vec<Option<QualType>>,
          symbol_refs: &mut FxHashMap<NodeId, SymbolId>,
          diagnostics: &mut Vec<Diagnostic>,
      ) -> Option<QualType> { ... }
  Call sites do the split:
      check_ident(name, span, id,
                  &ctx.table,
                  &mut ctx.expr_types,
                  &mut ctx.symbol_refs,
                  &mut ctx.diagnostics);
  Rust's disjoint-borrow rules make this compile. Zero runtime cost.

PATTERN B — Clone-and-release (when parameter lists get silly):
  Read what you need from the borrowed field, clone small values out, release
  the borrow, then mutate:
      let sym_ty = ctx.table.lookup(name)?.ty.clone();   // read scope ends
      ctx.set_type(id, sym_ty);                          // now mutable OK
  `QualType` is `Clone` (small struct, cheap clone). Prefer this pattern when
  a function needs broad access but only briefly reads from one field.

PATTERN C — Helper methods on SemaContext (for compound operations):
  Put sequential read/write logic behind `impl SemaContext`:
      impl SemaContext {
          fn record_symbol_use(&mut self, id: NodeId, name: &str)
              -> Option<QualType>
          {
              let (sym_id, ty) = {
                  let sym = self.table.lookup(name)?;
                  (sym.id, sym.ty.clone())
              }; // borrow of self.table ends here
              self.symbol_refs.insert(id, sym_id);
              self.set_type(id, ty.clone());
              Some(ty)
          }
      }

ANTI-PATTERNS — DO NOT USE:
  ❌ `RefCell<SymbolTable>` or `RefCell<Vec<Diagnostic>>`. Interior mutability
     hides borrow errors until runtime, panics in debug mode, and adds real
     overhead. It's the wrong tool for compiler state.
  ❌ `Rc<RefCell<...>>` or `Arc<Mutex<...>>` for sema data. Sema is
     single-threaded; synchronization is pure cost.
  ❌ Cloning the entire SymbolTable to work around a borrow conflict. That's
     a sign the call graph needs restructuring.
  ❌ `unsafe` to bypass the borrow checker. Never in forge_sema.

If you hit a borrow conflict you genuinely cannot resolve with A/B/C, stop
and restructure the call graph. It usually means a function is doing two
unrelated jobs.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Declaration analysis
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

analyze_declaration(decl: &Declaration, ctx: &mut SemaContext):

FUNCTION SPECIFIERS (C17 §6.7.4) — EXTEND Symbol FROM PROMPT 4.2:
Prompt 4.2 defined Symbol without function specifier flags. Add to Symbol:
    pub is_inline: bool,     // from `inline` keyword
    pub is_noreturn: bool,   // from `_Noreturn` keyword
    pub has_noreturn_attr: bool,  // from __attribute__((noreturn)), merged below
These are only meaningful for Function symbols; ignore on variables (warn).

When resolving DeclSpecifiers:
  - `inline` is a function specifier, not a storage class. Can combine with
    static/extern. Meaningful only on function declarations/definitions.
  - `_Noreturn` is a function specifier (C11). Same rules.
  - On a non-function declaration: warning "function specifier on non-function",
    do not error.
  - Multiple occurrences of the same specifier are OK (redundant, no warning).

LINKAGE IMPLICATIONS OF `inline` (C17 §6.7.4p7) — DEFERRED:
  Strict C rules distinguish:
    - plain `inline`: external inline definition, another TU must have the
      non-inline definition (or a call will fail to link).
    - `static inline`: TU-local inline, no external linkage.
    - `extern inline`: external definition; this TU provides one.
  For Phase 4: record the flags on Symbol but treat linkage as if `inline`
  were absent (so a `static inline foo` is internal-linkage as expected,
  plain `inline foo` keeps its normal external linkage). Codegen and Phase 5
  IR lowering will revisit this. Document with a TODO.

1. Resolve type specifiers → base QualType.
2. Determine storage class.
3. For each InitDeclarator:
   a. Resolve declarator → (name, final_type).
   b. Determine linkage:
      File scope, no storage  → External
      File scope, static      → Internal
      File scope, extern      → External
      Block scope, no storage → None
      Block scope, extern     → External
      Block scope, static     → None (static local)
   c. Redeclaration handling (use are_compatible from 4.1):
      - Same scope, same name, incompatible → error
      - Multiple externs, compatible → merge via composite_type
      - Tentative def + full def → the full def wins
      - Two full definitions → error
   d. Typedef → SymbolKind::Typedef (no storage allocated).
   e. Function → SymbolKind::Function.
   f. Otherwise → Variable.
   g. Initializer → check_initializer (Section 3).

_Static_assert at file scope:
   Evaluate condition via eval_icx_as_i64. If 0, push error with the message.
   File-scope _Static_assert must be handled here because it can appear among
   external declarations. (Block-scope handled in Prompt 4.6.)

Tentative definitions (C17 §6.9.2):
   Multiple `int x;` at file scope OK. At TU end, tentative defs without a
   real def become zero-initialized definitions.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Initializer type checking
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

check_initializer(init: &Initializer, target_type: &QualType, ctx: &mut SemaContext):

Simple (Expr):
  Type-check the expression. Check assignability. Insert implicit conversions.

Brace-enclosed (List):
  Array: each elem matches element type.
  Struct: each initializer matches next member, in declaration order.
  Union: first initializer matches first member (or the designated member).
  Designated initializers:
    .field → look up member
    [index] → evaluate via eval_icx_as_u64, check bounds for fixed-size arrays
  Excess initializers → warning.
  Nested braces → recurse.

String literal initializing char array:
  char s[] = "hello" → array becomes char[6]
  char s[3] = "hello" → warning: initializer truncated
  char s[6] = "hello" → OK

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Struct/union layout (with forward refs and anonymous members)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct StructLayout {
    pub tag: Option<String>,
    pub members: Vec<MemberLayout>,
    pub total_size: u64,
    pub alignment: u64,
    pub is_packed: bool,
    pub is_complete: bool,
    pub has_flexible_array: bool,
}

pub struct MemberLayout {
    pub name: Option<String>,                   // None for anonymous bit-field
                                                // or anonymous struct/union member
    pub ty: QualType,
    pub offset: u64,
    pub bit_field: Option<BitFieldLayout>,
    pub anon_members: Option<AnonMemberMap>,    // for C11 anonymous struct/union
}

pub struct AnonMemberMap {
    pub fields: HashMap<String, (u64, QualType)>,   // name → (offset_from_container, ty)
}

pub struct BitFieldLayout {
    pub width: u32,
    pub bit_offset: u32,
    pub storage_unit_size: u32,
}

TWO-PHASE LAYOUT for self-referential and mutually recursive structs:

Phase A — Register:
  When the parser gives us `struct node { ... }` or `struct node;`:
    If tag not yet declared, create a StructLayout with is_complete = false
    and insert into ctx.struct_defs, register tag in scope.
  This allows members that reference `struct node *` to resolve during Phase B.

Phase B — Complete:
  After Phase A, walk the member list and compute layout. Members can now
  reference the struct via pointer (incomplete pointee is OK for pointers).
  When done, set is_complete = true and update ctx.struct_defs[id].

  For `struct A { struct B *b; }` where struct B is not yet seen:
    Phase A registers A (incomplete). When resolving b's type, the tag B is
    not found — register an incomplete B and use it. When struct B is later
    defined, its Phase B completes that same entry.

  Cycle detection: a struct cannot CONTAIN itself by value (infinite size).
    `struct X { struct X x; }` → error. But `struct X { struct X *next; }` is fine.

Non-bit-field layout:
  For each member:
    1. Compute size and alignment (using explicit_align if set).
    2. offset = align_up(offset, alignment).
    3. Place member at offset.
    4. offset += size.
    5. Update max alignment.
  Final: total_size = align_up(offset, max_alignment).

Bit-field layout (GCC System V ABI):
  A bit-field of width W in a storage unit of type T:
    If W fits in current storage unit (and alignment matches), pack.
    Else start a new storage unit at T's alignment.
    Zero-width `int : 0;` forces next field to start at int boundary.
  Start simple: one storage unit per bit-field of the declared type's size.
  Pack only within a run of same-type bit-fields.

Flexible array member:
  Last member of form `T arr[];` — does NOT contribute to size.
  Record its offset; mark struct.has_flexible_array = true.

  FAM STRUCTURAL RESTRICTIONS (C17 §6.7.2.1p18) — VALIDATE at end of Phase B:
    (a) Struct cannot consist SOLELY of a FAM. There must be at least one
        other named member with complete type before it.
          struct bad1 { int data[]; };           → error
          struct ok   { int n; int data[]; };    → OK
    (b) A struct with a FAM cannot be an element of an array:
          struct ok items[10];                   → error
    (c) A struct with a FAM cannot be a non-last member of another struct,
        nor nested inside a union or another struct's non-last position:
          struct bad2 { struct ok first; int more; };   → error
          struct ok2  { int x; struct ok last; };        → OK (FAM at end)
          struct bad3 { struct ok inner; };              → also ok only if
                                                           bad3 itself would
                                                           then be FAM-bearing
                                                           and subject to the
                                                           same rules

  Validation flow:
    1. When computing struct layout, if the LAST member is incomplete-array
       of known element type, mark has_flexible_array = true.
    2. Validate (a): if has_flexible_array && members.len() == 1 → error.
    3. When a member's type is a struct with has_flexible_array:
       - If inside an array type for a member → error (b).
       - If the member is NOT last in the containing struct → error (c).
       - The containing struct thereby also becomes FAM-bearing (propagate).
    4. When declaring `struct ok items[N]` as a variable or member →
       check element type's has_flexible_array; if set → error.
  These checks prevent FAM structs from escaping their single-instance contract.

C11 anonymous struct/union members:
  A member with no name whose type is a struct/union type: that struct's
  members become accessible through the outer struct via name lookup.
    struct Outer {
        int x;
        union { int a; float b; };   // anonymous
    };
    Outer o; o.a = 1;   // accesses the anonymous union's member
  Record in AnonMemberMap so member lookup in expression analysis finds them.
  Nested anonymous allowed; flatten recursively.

Union layout: size = max(member sizes), align = max(member aligns),
              all offsets = 0.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Enum analysis
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct EnumLayout {
    pub tag: Option<String>,
    pub constants: Vec<(String, i64)>,
    pub underlying_type: Type,   // DETERMINED BY VALUE RANGE — not always Int
    pub is_complete: bool,
}

analyze_enum(def, ctx):
  First enumerator = 0 unless explicit.
  Subsequent = previous + 1 unless explicit.
  Explicit values evaluated via eval_icx_as_i64.
  Each enumerator added to the ORDINARY namespace (not tag) as EnumConstant.

  UNDERLYING TYPE SELECTION (C17 §6.7.2.2p4):
  The standard says the enum's underlying type is implementation-defined but
  must be able to represent all its enumerator values. Individual enumerator
  CONSTANTS still have type `int` (per the standard), but the enum TYPE itself
  may need to widen when values don't fit in int.

  Algorithm — after all enumerators are resolved, walk them and compute:
    min_val = min of all enumerator values
    max_val = max of all enumerator values
    any_negative = min_val < 0

  Choose the narrowest standard integer type containing [min_val, max_val]:
    any_negative:
      fits in i32 → Int (signed)            // the common case
      fits in i64 → Long (signed, LP64)
      else → error (truly exotic; shouldn't happen with i64 eval)
    all non-negative:
      fits in i32 max (INT_MAX) → Int (signed)
      fits in u32 max (UINT_MAX) → Int (unsigned)
      fits in i64 max → Long (signed)
      else → Long (unsigned)

  Matches GCC/Clang behavior. Record on EnumLayout.underlying_type.
  Note: this affects sizeof(enum X) and the type of an enum-typed lvalue in
  arithmetic (after integer promotion it still promotes to int in most cases,
  but that's handled in Prompt 4.1's integer_promotion for rank < int).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 6 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Declarations:
  int x; at file scope → Variable, External
  static int x; → Internal
  extern int x; → External
  typedef int MyInt; MyInt y; → y is Int
  int x = 5;  → init type-checked
  int x = 3.14; → implicit FloatToInt, warning
  Multiple extern int x; → OK
  int x; int x; at file scope → OK (tentative)
  int x; float x; → error

Self-referential and mutual:
  struct node { struct node *next; }; → OK
  struct A { struct B *b; }; struct B { struct A *a; }; → OK
  struct X { struct X x; } → error (contains itself by value)

Struct layout (x86-64):
  { char a; int b; }       → size=8, align=4 (1+3+4)
  { int a; char b; }       → size=8, align=4 (4+1+3)
  { char a,b,c; }          → size=3, align=1
  { double d; int i; }     → size=16, align=8 (8+4+4)
  { char a; }              → size=1, align=1
  { }                      → size=0, align=1 (GCC ext)

Unions:
  { int i; double d; }     → size=8, align=8

Anonymous member (C11):
  struct S { int x; union { int a; float b; }; };
    s.a, s.b accessible.

Flexible array:
  struct S { int n; int data[]; }; → size=4 (data not counted)
  struct S { int data[]; };        → error: FAM alone (§6.7.2.1p18)
  struct S { int n; int data[]; } arr[10]; → error: array of FAM struct
  struct Outer { struct S first; int x; }; → error: FAM struct not last
  struct Outer { int x; struct S last; };  → OK: FAM struct at end
    Note: Outer itself now has_flexible_array; same rules propagate.

Enum:
  { A, B, C } → 0,1,2
  { X=10, Y, Z=5 } → 10,11,5
  Enumerators in ordinary namespace.

Enum underlying type widening:
  { A, B, C }                         → Int (fits in i32)
  { NEG = -1, POS = 1 }               → Int (negative present, fits in i32)
  { BIG = 3000000000 }                → unsigned Int (exceeds INT_MAX, no neg)
  { HUGE = 5000000000 }               → Long (exceeds u32 max)
  { NEG = -1, HUGE = 5000000000 }     → Long (neg + large)
  sizeof(enum { BIG = 3000000000 })   → 4 (unsigned int on LP64)
  sizeof(enum { HUGE = 5000000000 })  → 8 (long on LP64)

File-scope _Static_assert:
  _Static_assert(sizeof(int) == 4, "int must be 4"); → OK on x86-64
  _Static_assert(sizeof(int) == 8, "wrong"); → error with that message
```

### Prompt 4.4 — Expression type checking (part 1: basics, lvalue, sizeof/alignof)

```
Implement expression type checking for basic expressions and lvalue analysis.

Split into two prompts. This prompt: literals, identifiers, member access,
subscript, sizeof, _Alignof, lvalue analysis, default decay conversions.
Prompt 4.5: operators, calls, casts, compound literals, _Generic, assignment.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Lvalue analysis
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Lvalues:
  Identifier → var/param: lvalue
  *ptr: lvalue
  obj.member: lvalue iff obj is lvalue (AND obj is not rvalue struct)
  ptr->member: always lvalue
  arr[i]: lvalue
  string literal: lvalue (stored in memory)
  compound literal: lvalue

NOT lvalues:
  integer/float/char literals (other than strings)
  function call results
  arithmetic/comparison results
  cast results
  post-increment/decrement (pre-increment is lvalue? in C: no, both post and pre
    increment/decrement produce rvalues. Different from C++.)
  ternary — skip the rare lvalue case

Modifiable lvalue: lvalue AND not const AND not array AND complete type AND
  (if struct/union) no const member recursively.

is_lvalue(node_id, ctx) -> bool
is_modifiable_lvalue(node_id, ctx) -> bool

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Default conversions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

apply_default_conversions(expr_id, ctx):
  1. LvalueToRvalue if expr is lvalue and not in an lvalue-preserving context.
  2. ArrayToPointer if type is array.
  3. FunctionToPointer if type is function.
  Record conversions in ctx.implicit_convs; update expr_types.

Contexts that SUPPRESS these conversions:
  - Operand of & (address-of): no L2R, no decay.
  - Operand of sizeof/_Alignof: no L2R, no array decay, no function decay.
  - LHS of assignment (incl compound): no L2R.
  - String literal initializing char array: no array decay.
  - ++/--: L2R suppressed on operand (it's modified in place).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Literal types
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Integer literal (C17 §6.4.4.1):
  Unsuffixed decimal: first of int, long, long long that fits.
  Unsuffixed octal/hex: first of int, unsigned int, long, unsigned long,
                         long long, unsigned long long that fits.
  U: first of unsigned int, unsigned long, unsigned long long.
  L: first of long, long long (unsigned variants for hex/octal).
  UL: first of unsigned long, unsigned long long.
  LL: long long. ULL: unsigned long long.

Float literal: double by default. F → float. L → long double.

Character literal: type is int. Value is the char code.

String literal: type is char[N+1]. Prefixes:
  L"..." → wchar_t[N+1]
  u"..." → char16_t[N+1]
  U"..." → char32_t[N+1]
  u8"..." → char[N+1] (UTF-8)
(char16_t/char32_t typedefs need to be pre-seeded; see Prompt 4.7.)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Identifier
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

check_ident(name, span, ctx, is_call_target):
  1. Lookup in symbol table.
  2. Not found:
     If is_call_target: implicitly declare as `int name()` (warning,
       "implicit declaration of function 'name'"). This is C89 legacy that
       real-world headers rely on.
     Else: error "use of undeclared identifier 'name'".
  3. Record symbol_refs[node_id] = symbol.id.
  4. Type = symbol's type.
  5. Lvalue iff Variable or Parameter (not EnumConstant, not Function, not Typedef).
  6. Default conversions applied by caller context.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Member access, subscript, sizeof, _Alignof
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

check_member_access(obj, name, is_arrow, ctx):
  If is_arrow: apply default conversions to obj; obj type must be pointer to
    struct/union; dereferenced struct/union is the container.
  Else: obj type must be struct/union (no conversion).
  Look up name in the struct/union's member namespace. If the struct has
  anonymous members, search their AnonMemberMap too.
  Not found → error "no member named '<name>' in 'struct <tag>'".
  Result type = member.ty, with any qualifiers of obj merged in (const struct
  → const member).
  Lvalue: true for arrow; for dot, true iff obj is lvalue.

check_subscript(arr, idx, ctx):
  Apply default conversions to both.
  Exactly one must be pointer to complete type, other must be integer.
  Result type = pointee. Always lvalue.

check_sizeof(operand, ctx):
  Operand suppresses L2R and decays (except for VLA, which IS evaluated).
  If type-name: size_of(type).
  If expr: determine type without evaluating, size_of(type).
  sizeof("hello") = 6. sizeof(array) = full array bytes.
  Result type: size_t. Not lvalue.
  If operand type is incomplete, function, or bit-field → error.

  VLA SIDE EFFECTS (C17 §6.5.3.4p2) — CRITICAL:
  If the operand (type-name OR expression) involves a VLA, the size expression
  IS evaluated at runtime, and side effects in it execute. Examples:
      int n = 5;
      sizeof(int[n++]);    // n → 6; size = old_n * sizeof(int) = 20
      int vla[n];          // n is now 6
      sizeof(vla);         // evaluates (in fact reads the captured dimension)

  TAST REPRESENTATION:
      pub enum SizeofKind {
          Constant(u64),                   // non-VLA: compile-time size
          RuntimeVla { expr_nodes: Vec<NodeId> },
                                           // VLA: list of size expressions
                                           // that must execute at runtime
      }
  Record on a side table: ctx.sizeof_kinds: HashMap<NodeId, SizeofKind>.

  Detection:
    A type is "VLA-involving" if ANY of its array dimensions (at any depth) is
    ArraySize::Variable. Recurse through pointers, arrays, and struct members.
    Note: pointers-to-VLA are common (`int (*p)[n]`) and still VLA-involving.

  For sizeof(expr) where expr's type is VLA-involving:
    - Do NOT evaluate the expression itself (sizeof still doesn't run expr).
    - DO record the size-expression NodeIds so Phase 5 can emit their
      evaluation.
  For sizeof(type-name) where type-name is VLA-involving:
    - Record the size-expression NodeIds from the type-name's array bounds.

  For non-VLA cases, SizeofKind::Constant(size_of(type)) with no runtime work.
  This representation is consumed by Phase 5 IR lowering.

check_alignof(operand, ctx):
  _Alignof(type-name) is the standard C11 form.
  __alignof__(type-name) and __alignof__(expr) are GCC extensions found in
  glibc headers. Accept all three:
    - _Alignof(type-name)           → align_of(type)
    - __alignof__(type-name)        → align_of(type)
    - __alignof__(expr)             → align_of(expr's type)  [type determined
                                       without evaluating the expression,
                                       same no-L2R/no-decay rule as sizeof]
  Result type: size_t. Not lvalue.
  If operand type is incomplete or function → error.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 6 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Lvalues:
  int x; → x lvalue, modifiable
  const int x = 5; → lvalue, NOT modifiable
  *p, arr[0], s.m, p->m → lvalue
  42, a+b, f() → not lvalue

Literals:
  42 → int, 42U → unsigned int, 42L → long, 42ULL → unsigned long long
  2147483648 → long (LP64)
  0xFFFFFFFF → unsigned int
  3.14 → double, 3.14f → float, 3.14L → long double
  'a' → int (value 97)
  "hello" → char[6]

Identifiers:
  declared int x then x → Int, lvalue
  undeclared y → error
  undeclared foo() → warning, implicit int foo()

Members:
  struct S { int x; } s; s.x → int, lvalue
  struct S *p; p->x → int, lvalue
  s.nonexistent → error
  Anonymous union: struct { int x; union { int a; float b; }; } s; s.a works.

Subscript:
  int arr[10]; arr[0] → int, lvalue
  int *p; p[0] → int, lvalue
  "hello"[0] → char after decay

sizeof:
  sizeof(int) → 4
  sizeof("hello") → 6   (no decay!)
  sizeof(int[10]) → 40
  sizeof(struct { int a; char b; }) → 8

sizeof with VLA (runtime evaluation):
  int n = 5; sizeof(int[n]);
    → SizeofKind::RuntimeVla { expr_nodes: [node_id_of_n] }
  int n = 5; sizeof(int[n++]);
    → RuntimeVla with n++ expression recorded (Phase 5 emits evaluation)
  void f(int m) { int vla[m]; sizeof(vla); }
    → RuntimeVla referring to m (captured at vla's declaration)
  sizeof(int *[n])  (array of pointers, VLA dim)
    → RuntimeVla
  sizeof(int (*)[n])  (pointer to VLA)
    → RuntimeVla (type-name still involves VLA dim)
  sizeof(int[5])    → Constant(20)  (not a VLA)

_Alignof:
  _Alignof(int) → 4
  _Alignof(double) → 8
  _Alignof(struct { char c; int i; }) → 4
  __alignof__(int) → 4                    (GCC syntax)
  __alignof__(some_int_var) → 4           (GCC expression form)
  __alignof__("hello") → 1                (char array alignment, no decay)
```

### Prompt 4.5 — Expression type checking (part 2: operators, calls, casts, _Generic)

```
Complete expression type checking: all operators, function calls, casts,
compound literals, ternary, assignment, _Generic, comma.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Arithmetic and unary
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Binary + - * / %:
  Apply default conversions to both.
  Both arithmetic → usual arithmetic conversions → common type.
  % → integer only.
  + and -: also pointer arithmetic (Section 2).

Unary + -: operand arithmetic, integer promotion, result = promoted type.
Unary ~: operand integer, integer promotion, result = promoted type.
Unary !: operand scalar, result = Int(signed).

Address-of &:
  Operand must be lvalue (not register, not bit-field).
  Result = pointer to operand type.

Dereference *:
  Operand must be pointer type.
  Error if pointee is incomplete AND result is used for value.
  Result type = pointee. Result IS lvalue.

  FUNCTION POINTER DEREFERENCE (C17 §6.5.3.2p4):
  Dereferencing a function pointer yields a function designator, which is
  NOT an error and NOT a missing-operand case. Function designators then
  decay back to function pointers via FunctionToPointer in any value context.
  This means `(*f)()`, `(**f)()`, `(****f)()` are all valid and equivalent
  to `f()` — common in signal handlers and dispatch tables.

  Implementation note: when * is applied to a pointer-to-function, the result
  type is Function (NOT lvalue — function designators are not lvalues per
  §6.3.2.1p1). Apply_default_conversions then triggers FunctionToPointer,
  restoring the pointer type. The cycle is finite because each * consumes
  one syntactic dereference; there is no infinite loop at the type level.
  Do NOT write code that rejects * on function-pointer types.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Pointer arithmetic
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Addition:
  ptr + int → ptr (pointee must be complete)
  int + ptr → ptr (commutative)

Subtraction:
  ptr - int → ptr
  ptr - ptr → ptrdiff_t (Long, signed on LP64). Both must point to compatible
    complete types.

Comparison:
  ptr == ptr: compatible types, or one is void*
  ptr == null_pointer_constant (literal 0 or (void*)0) → OK
  ptr <,>,<=,>=: same rules.

++ / --:
  Pointer arithmetic on pointer types.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Comparison, bitwise, shift, logical
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Relational < > <= >=:
  Both arithmetic → usual arithmetic conversions, result Int(signed).
  Both pointers to compatible types → Int(signed).

Equality == !=:
  Arithmetic: usual conversions.
  Pointer + compatible pointer, pointer + void*, pointer + null constant.

Bitwise & | ^:
  Both integer. Usual arithmetic conversions. Common type.

Shift << >>:
  Both integer. Integer promotion on EACH operand INDEPENDENTLY.
  Result type = promoted LEFT operand type (NOT the common type).
  Warn if right operand is >= width of left operand's promoted type.

Logical && ||:
  Both scalar. Result = Int(signed). Short-circuit (remember for IR).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Assignment
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Simple assignment (=):
  LHS must be modifiable lvalue.
  RHS assignability to LHS type:
    arithmetic ← arithmetic (with conversion)
    pointer ← compatible pointer (qualification add OK)
    pointer ← void* or void* ← pointer
    pointer ← null pointer constant
    struct/union ← same struct/union
    _Bool ← any scalar
  Insert implicit conversions.
  Result type = LHS type UNQUALIFIED. Result NOT lvalue.

Compound assignment (+=, -= etc):
  Equivalent to `LHS = LHS op RHS`, LHS evaluated once.
  Rules combine binary-op rules and assignment rules.

Pre/post ++/--:
  Operand: modifiable lvalue, arithmetic or pointer.
  Result type = operand type. Result NOT lvalue (both pre and post in C).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Function calls
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

check_call(callee, args, ctx):
  Type-check callee; function-to-pointer decay.
  Callee type must be pointer to function.
  Extract the function type.

  If is_prototype == true:
    Check argument count (equal to params, or ≥ params if variadic).
    For each fixed arg: must be assignable to param type; insert conversion.
    For variadic args beyond fixed params: DEFAULT ARGUMENT PROMOTIONS:
      integer promotion (char/short → int)
      float → double
    This is why printf("%f", 3.0f) works.
  Else (unprototyped `int f()`):
    Apply default argument promotions to ALL arguments.
    Cannot validate count/types — warning.

  Result type = return_type. Not lvalue.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 6 — Cast, ternary, compound literal, _Generic, comma
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Cast (T)e:
  Valid:
    arithmetic ↔ arithmetic
    integer ↔ pointer
    pointer ↔ pointer (including void*)
    pointer → _Bool
    anything → void (explicit discard)
  Invalid:
    struct/union as source or target
    array type as target
    function type as target
  Result type = T. Not lvalue.

Ternary c ? a : b:
  c must be scalar.
  If a and b arithmetic: usual conversions.
  If both pointers to compatible types: composite pointer.
  If one pointer, other null constant: pointer.
  If one pointer, other void*: void* (taking the union of qualifiers).
  If both void: void.
  If both same struct/union: that type.
  Result not lvalue (skip the rare lvalue case).

Compound literal (T){init}:
  Check initializer against T.
  Result type = T. Result IS lvalue.

_Generic(controller, assoc1, assoc2, ..., default: expr):
  Determine type of controller WITHOUT applying L2R conversion, but WITH
  array→pointer and function→pointer decay (per C17 §6.5.1.1p3). Strip qualifiers.
  For each association (type-name: expr):
    Check compatibility with controller's type using are_compatible_unqualified.
  Select the single match, or default.
  Zero matches + no default → error.
  Multiple matches → error.
  ONLY the selected expression is type-checked and its type becomes the result.
  Unselected associations are not type-checked (C17 §6.5.1.1p2).
  Not lvalue.

Comma a, b:
  Type-check a (discard), type-check b.
  Result type = b's type. Result is lvalue iff b is lvalue (some debate; C says
  not lvalue, GCC/Clang agree — go with not lvalue).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 7 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Arithmetic:
  int+int → int;   char+char → int;  int+long → long;  int+double → double
  unsigned int + int → unsigned int;   int % float → error

Pointer arithmetic:
  int*+int → int*;  int*-int* → long (ptrdiff_t)
  void*+int → error (incomplete pointee)
  int*+int* → error

Shift:
  (char)1 << 20 → int (LEFT operand's promoted type)
  1ULL << 10 → unsigned long long

Assignment:
  int x; x = 5 → OK
  const int x; x = 6 → error
  int *p; p = 0 → OK (null)
  int x; float y; x = y → OK with warning (float→int)
  struct S s1, s2; s1 = s2 → OK

Function calls:
  int f(int); f(42) → OK
  int f(int); f(3.14) → OK w/ double→int
  int f(int); f(1, 2) → error
  int f(int, ...); f(1, 'a', 3.14f) → OK, promotions
  undeclared foo() → warning, implicit int foo()

Cast:
  (int)3.14 → int
  (void*)p → void*
  (int)p → int (ptr→int)
  (struct S)42 → error

Function pointer dereference cycle (all equivalent to f()):
  void f(void); (*f)();     → OK, f decays to ptr, *ptr yields function,
                               which decays back to ptr, then call
  void f(void); (**f)();    → OK, two dereference cycles
  void f(void); (****f)();  → OK, still valid
  typedef int (*fp)(int); fp g; (*g)(42);  → OK, call through pointer

Ternary:
  c ? 1 : 2 → int
  c ? 1 : 2.0 → double
  c ? ptr : 0 → pointer
  c ? iptr : fptr → error

_Generic:
  _Generic(42, int: 1, default: 0) → 1
  _Generic(3.14, double: "d", float: "f") → "d" (only this is type-checked)
  int *p; _Generic(p, int *: 1, default: 0) → 1

Compound literal:
  (int){42} → int, lvalue
  (struct S){.x = 1} → struct S, lvalue

Comma:
  (1, 2, 3) → int value 3
```

### Prompt 4.6 — Statement analysis + block-scope _Static_assert

```
Implement statement type checking and function body analysis.
(File-scope _Static_assert was handled in Prompt 4.3. Block-scope version here.)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — FnContext (label namespace lives here)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct FnContext {
    pub return_type: QualType,
    pub in_loop: bool,
    pub in_switch: bool,
    pub switch_stack: Vec<SwitchInfo>,   // nested switches
    pub labels_defined: HashMap<String, Span>,    // label namespace (4th C namespace)
    pub labels_referenced: Vec<(String, Span)>,
}

pub struct SwitchInfo {
    pub controlling_type: Type,          // promoted integer type
    pub case_values: HashSet<i64>,
    pub has_default: bool,
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Statement analysis
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

analyze_stmt(stmt, ctx, fn_ctx):

Compound: push Block scope, analyze each block item, pop.
If: condition must be scalar. Analyze both branches.
  Warn on `if (x = 5)` suggesting `==`.
While / DoWhile: scalar cond. Body: in_loop=true (save and restore).
For: push scope for init-decl if present. Cond scalar. Body in_loop=true.
Switch:
  Controlling expr must be integer (after promotion).
  Push SwitchInfo. in_switch=true.
  Analyze body. Pop SwitchInfo.
Case:
  Must be inside switch (non-empty switch_stack).
  Value via eval_icx_as_i64.
  Duplicate case value in this switch → error.
Default:
  Must be inside switch.
  Duplicate default → error.
Return:
  Void func + return expr with non-void type → error.
  Non-void func + bare return → warning.
  Non-void func + return expr → type-check, must be assignable to return_type.
Break: in_loop OR in_switch → OK. Else error.
Continue: in_loop → OK. Else error. (Switch alone does NOT satisfy continue.)
Goto: record in labels_referenced.
Label: add to labels_defined. Duplicate → error.
ExprStmt: type-check the expression (result discarded).
DeclStmt: analyze_declaration in current block scope.
_Static_assert: evaluate via eval_icx_as_i64; if 0, error with message.

At function end:
  For each labels_referenced entry, verify it exists in labels_defined.
  Missing → error "use of undeclared label '<name>'".
  For non-void funcs: if last statement is not a return, warn "control may
  reach end of non-void function". (Simple heuristic; full CFG analysis later.)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Function definition analysis
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

analyze_function_def(func, ctx):
  1. Resolve return type and parameter types (reusing resolve_declarator).
  2. Register function in ordinary namespace.
     If previous declaration exists, merge via composite_type.
  3. Push Function scope.
  4. Add parameters to scope (check for duplicate parameter names).
  5. Build FnContext with return_type.
  6. Analyze body (compound statement) in Function scope.
  7. Resolve labels.
  8. Pop Function scope.

  PARAMETER / OUTERMOST BLOCK SCOPE (C17 §6.2.1p4) — IMPORTANT:
  Function parameters and the outermost block of the function body share
  the SAME scope. Do NOT push a new Block scope when analyzing the
  function body's top-level compound statement — reuse the Function scope
  opened in step 3. Push Block scopes only for NESTED compound statements
  inside the body.

  Consequence (and the reason this matters):
      int f(int x) {
          int x = 5;          // error: redeclaration of 'x'
          { int x = 5; }      // OK: inner block, legal shadowing
          return 0;
      }
  The first `int x = 5;` is caught by the existing "duplicate in same scope"
  check from Prompt 4.2's SymbolTable::declare, because x (parameter) and x
  (local) land in the same scope. No special "shadowing" rule is needed —
  the duplicate-detection path is sufficient, provided steps 3–6 above
  don't accidentally open a second scope.

  Suggested implementation: when the parser's FunctionDef carries a
  CompoundStmt for the body, analyze_function_def directly processes that
  CompoundStmt's block items WITHOUT going through the generic
  analyze_stmt(CompoundStmt) path that would push a fresh Block scope.
  Nested CompoundStmts inside still go through the generic path.

K&R style old declarations:
  If the parser produces an old-style definition (identifier list, declarations
  before body), emit an error pointing at the parameter list: "old-style
  (K&R) function definition is not supported; use a prototype".

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Translation unit analysis
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

analyze_translation_unit(tu, target) -> SemaContext

1. Build SemaContext with file-scope SymbolTable.
2. Pre-seed builtin typedefs (see Prompt 4.7 for full list):
   __builtin_va_list, __int128 stub, _Float16/32/64/128 stubs.
3. For each ExternalDeclaration:
   - Declaration → analyze_declaration
   - FunctionDef → analyze_function_def
   - _Static_assert → evaluate here (file scope)
4. At TU end:
   - Resolve tentative definitions: mark them on the Symbol as "this TU
     provides a definition" (e.g., Symbol.is_defined = true) so downstream
     phases know there's something to emit. Do NOT synthesize a fake
     initializer here — simply flag that the symbol needs a zero-initialized
     definition.

     The ACTUAL choice between emitting `.comm` (common symbol, collapsible
     by the linker across TUs — GCC default) and `.bss` (zero-initialized
     with external linkage, yields multi-definition errors on conflict
     — GCC's -fno-common mode) is Codegen's decision (Phase 7), not
     Sema's. Phase 4 records intent; Phase 7 lowers it. This preserves
     flexibility to match GCC default behavior later.

   - Warn on unused static symbols (optional, skip for v1).
5. Return context — diagnostics accessible via ctx.diagnostics.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Conditions:
  if (int) OK; if (ptr) OK; if (struct) error.

Loops:
  while(1) OK; break outside loop → error; continue outside loop → error;
  break in switch OK; continue in switch alone → error.

Switch:
  switch(int) OK; switch(3.14) error; dup case error; dup default error.

Return:
  void f() { return 5; } → error
  int f() { return; } → warning
  int f() { return 3.14; } → OK w/ float→int

Goto/label:
  goto end; end: return; → OK
  goto missing; → error
  dup: dup: → error

Block-scope _Static_assert:
  void f() { _Static_assert(sizeof(int) == 4, "!"); } → OK
  void f() { _Static_assert(0, "nope"); } → error

Parameter + outermost block scope (C17 §6.2.1p4):
  int f(int x) { int x = 5; return x; }
    → error: redeclaration of 'x' (params and outer block share scope)
  int f(int x) { { int x = 5; return x; } }
    → OK: inner block shadowing is legal
  int f(int x, int x) { ... }
    → error: duplicate parameter name

Full function:
  int add(int a, int b) { return a + b; } → OK, returns int
  Function with if/else + for + switch + nested struct access → type-checks.
```

### Prompt 4.7 — GNU extension semantics + driver integration

```
Add semantic handling for GNU extensions found in system headers, and wire
sema into the driver.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — __typeof__ resolution
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

TypeSpecifierToken::TypeofExpr(expr):
  Type-check expr. Resulting type (with qualifiers) becomes the specifier.
  Expression is NOT evaluated.

TypeSpecifierToken::TypeofType(type_name):
  Resolve type name. Use directly.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — __builtin_va_list and varargs builtins
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Pre-seed:
  typedef __opaque_va_list_storage __builtin_va_list;
  where __opaque_va_list_storage is a hidden struct type. On x86-64 System V,
  va_list is actually `__va_list_tag[1]`, but we only need acceptance for now.

Builtins: add as pre-declared functions in the ordinary namespace:
  void __builtin_va_start(__builtin_va_list, ...);   // variadic signature
  void __builtin_va_end(__builtin_va_list);
  void __builtin_va_copy(__builtin_va_list, __builtin_va_list);

__builtin_va_arg is type-dependent (takes a type-name as second argument).
Phase 3 must parse this as a special form, not a regular call. If Phase 3
parsed it as a call-like expression with a parse-time type argument, add
a dedicated checker: result type = that type, validate first argument is
__builtin_va_list. If Phase 3 couldn't parse it, add a TODO and allow the
parser to accept any call-like form; sema returns Int as a placeholder.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — __attribute__ handling (minimal)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

The parser collects __attribute__ lists. In sema, process only:
  aligned(N)  → set QualType.explicit_align = Some(N) on the declared type
  packed      → set StructLayout.is_packed = true (forces alignment 1)
  noreturn    → record on Symbol (used later to suppress "missing return" warn)
All others: silently ignore. Must NEVER error on an unknown attribute in system
headers.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Other GCC extensions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

__int128: treat as LongLong for Phase 4 (size/align approximate). TODO for real
  128-bit support later.

_Float16/32/64/128: pre-seed as typedefs:
  typedef float _Float32;
  typedef double _Float64;
  typedef long double _Float128;
  typedef float _Float16;   // approximate
These are wrong in strict IEEE semantics but sufficient for header parsing.

__builtin_offsetof(struct-type, member):
  Look up member. Compute its byte offset via struct layout.
  Result type = size_t.
  Evaluatable in eval_icx (add it there as a special form).

__builtin_types_compatible_p(t1, t2):
  Evaluate are_compatible_unqualified(t1, t2). Result Int (0 or 1).
  Also evaluatable in eval_icx.

__extension__: parser drops it; nothing to do in sema.

__alignof__ (both type-name and expression form): handled in Prompt 4.4's
  check_alignof alongside standard _Alignof. Listed here for cross-reference
  because it surfaces in glibc headers. Also evaluatable in eval_icx if the
  operand's alignment is statically known.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Builtin typedefs pre-seeding
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

At the start of analyze_translation_unit, declare:
  __builtin_va_list (as above)
  char16_t, char32_t (unsigned short/unsigned int on Linux)
  Integer builtins above.

Many other identifiers (size_t, int32_t, etc.) come from system headers —
we do NOT pre-seed those here. If they're not in headers, it's a user error.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 6 — Driver integration
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

In forge_driver:
  Pipeline: lex → preprocess → parse → sema.
  `forge check file.c` runs all four; reports all accumulated diagnostics.
  `forge check --dump-types file.c` prints each declaration and the type of
    each expression at its span (for debugging).
  `forge parse file.c` still works (parse only).
  `forge -E file.c` still works (preprocess only).
  Exit code non-zero if any error-level diagnostic was emitted.

  KNOWN LIMITATION — macro expansion backtraces:
  As of v5, Span carries only (start, end). Diagnostics from code produced
  by macro expansion will show the expansion-result location, not "in
  expansion of macro M called at line N". A separate Phase 2 follow-up
  (multi-file Span with FileId + ExpansionId) resolves this; it lands
  independently of Phase 4. During Phase 4 development, debugging a
  macro-induced type error may require manually inspecting the preprocessed
  output (`forge -E`) to find the original macro call site.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 7 — System header smoke test
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

THE BIG TEST — tests/lit/sema/headers_smoke.c:
  #include <stdio.h>
  #include <stdlib.h>
  #include <string.h>
  #include <stdint.h>

  int main(void) {
      int x = 42;
      printf("hello %d\n", x);
      char *s = malloc(100);
      if (s) {
          strcpy(s, "world");
          printf("%s\n", s);
          free(s);
      }
      return 0;
  }

Run lex → preprocess → parse → sema. ASSERT ZERO error-level diagnostics.

Likely initial failures:
  - Missing builtin typedef
  - __builtin_va_arg parsing mismatch
  - Some GNU attribute in a header we haven't seen
  - An implicit function from system headers
Iterate until it passes.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 8 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

__typeof__(42) x; → x is int
__typeof__(int *) p; → p is int*
Code using __builtin_va_list → accepted
__builtin_offsetof(struct S {int a; int b;}, b) evaluated as 4
__builtin_types_compatible_p(int, int) → 1
__builtin_types_compatible_p(int, long) → 0
System header smoke test passes
```

### Prompt 4.8 — Full validation

```
Comprehensive validation of forge_sema before Phase 5.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 1 — Code audit
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

A. unwrap/expect audit in forge_sema → replace with ? / errors, or justify
   with a comment explaining why the unwrap is infallible.
B. TODO/FIXME audit → resolve or link to a tracking issue.
C. cargo clippy --all-targets -- -W clippy::pedantic → clean (warnings OK,
   no denies beyond -D warnings).
D. Dead code → remove or document.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 2 — Completeness matrix
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Fill every row with Test?/Pass?:

TYPE SYSTEM:
  All integer types (char, short, int, long, long long)
  Signed/unsigned variants
  Float, double, long double
  Pointer types
  Array types (fixed, incomplete, VLA)
  Function types (prototyped and unprototyped)
  Struct (including self-referential and mutually recursive)
  Union
  Enum (with underlying type widening per value range)
  Qualified (const, volatile, restrict, atomic)
  Typedef chains
  sizeof for all types (constant + runtime VLA)
  _Alignof for all types
  _Alignas affecting layout

TYPE SPECIFIER RESOLUTION:
  All valid combinations
  Invalid combinations → errors
  Any-order specifiers
  Typedef + qualifiers
  Function specifiers (inline, _Noreturn) recorded
  Array parameter qualifiers (T[const N], T[restrict], T[static N]) transferred

COMPATIBILITY / COMPOSITE:
  int ~ int
  Unprototyped vs prototyped function compatibility
  Array with/without size composite
  Tag-based struct compatibility (distinct tags incompatible)

IMPLICIT CONVERSIONS (each recorded in implicit_convs):
  LvalueToRvalue, ArrayToPointer, FunctionToPointer,
  IntegerPromotion, ArithmeticConversion,
  IntToFloat, FloatToInt, FloatConversion,
  PointerToBoolean, NullPointerConversion,
  IntegerToPointer, PointerToInteger,
  QualificationConversion, BitFieldToInt

EXPRESSIONS:
  All binary arithmetic
  All comparisons
  All logical
  All bitwise
  Shift (independent promotion; left-type result)
  Pointer arithmetic (all forms)
  Assignment (simple + compound)
  Pre/post ++/--
  Function calls (prototyped, unprototyped, variadic with default promotions)
  Cast
  sizeof (expr + type)
  sizeof with VLA (runtime, side effects preserved)
  _Alignof (type)
  __alignof__ (type and expr forms, GCC)
  Ternary
  Comma
  Member access (. and ->, including anonymous members)
  Subscript
  Address-of and dereference (including function pointer cycles)
  Compound literals
  _Generic selection (single-branch type-checking)

DECLARATIONS:
  Variable, function, typedef, struct/union/enum def
  Initializer type checking
  Designated initializer checking
  Storage class validation
  Linkage rules (inline semantics deferred to codegen)
  Tentative definitions
  File-scope _Static_assert
  _Alignas
  Flexible array member structural restrictions (§6.7.2.1p18)

STATEMENTS:
  If/while/for/do-while condition scalar check
  Switch expr integer check
  Return type compatibility
  Break/continue context checking
  Goto/label validation (label namespace)
  Duplicate case/default detection
  Block-scope _Static_assert
  Parameter / outermost-block same-scope redeclaration (§6.2.1p4)

CONST CORRECTNESS:
  Assignment to const → error
  Pointer-to-const vs const-pointer
  Struct with const member cannot be assigned
  Array parameter const from `T[const]` on the pointer itself

QUALIFIER PRESERVATION (for Phase 5 downstream):
  const roundtrip through composite_type, initializer check, casts
  volatile survives type manipulations (lvalue-to-rvalue preserves it on
    the access path even if result type drops it, per C17 §6.3.2.1p2)
  restrict survives on pointer params through calls and assignments
  _Atomic survives on the QualType when appearing in declarations
  A function-call result strips qualifiers from the TOP level of the
    return type (C17 §6.5.2.2p5) — qualifiers on pointee types are kept

ERROR MESSAGES (span + clear text):
  use of undeclared identifier
  incompatible types in assignment (uses QualType::to_c_string)
  too many/few arguments to function
  subscript of non-pointer
  member access on non-struct
  break/continue context
  duplicate case/default/label
  FAM-alone / FAM-in-array / FAM-not-last errors

GNU EXTENSIONS:
  __typeof__ / __typeof resolves
  __builtin_va_list accepted
  __builtin_offsetof / __builtin_types_compatible_p evaluate
  __attribute__ accepted (aligned/packed affect layout, noreturn recorded)
  System header smoke test passes

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 3 — Stress tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

1. 100 local vars in one function — all tracked
2. 50 nested scopes — shadowing correct
3. Struct with 50 members — layout correct
4. Function with 20 parameters — all type-checked
5. 10 levels of pointer indirection
6. Expression tree depth 100 — types resolve
7. Enum with 100 constants — values correct
8. Multiple TUs referencing same extern — no false conflicts
9. 30+ valid type-specifier combinations
10. Typedef chain of depth 10 — resolves
11. Mutually recursive structs (3-way cycle via pointers)
12. Anonymous union inside struct inside union
13. _Generic with 20 associations
14. Enum spanning full i64 range (underlying type picks unsigned long)
15. Nested FAM: struct with FAM as last member of another struct-with-FAM
16. VLA-of-pointer-to-VLA sizeof expression (runtime kind correctly recorded)
17. 6-level function-pointer dereference chain: (*****f)() type-checks

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 4 — Real-world test
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Hand-written test program (50+ lines) exercising:
  structs, enums, typedefs, function pointers,
  arrays with designated initializers,
  pointer arithmetic, casts, sizeof,
  switch/case, for loops, if/else,
  variadic function calls.
Run full pipeline. Assert zero errors.

System header sema — each must pass end-to-end:
  <stdio.h> + simple main
  <stdlib.h> + malloc/free
  <string.h> + strcpy/strlen
  <stdint.h> + uint32_t usage

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 5 — Performance
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Measure full pipeline (lex + preprocess + parse + sema).
Baseline from Phase 3: parser handles stdio.h in 14ms.
Sema budget on top should be comparable (type checking adds maps + lookups).

HASHING SANITY:
  Verify that forge_sema is using FxHashMap everywhere that keys by NodeId
  or small integer types. A quick grep:
      grep -r "std::collections::HashMap\|HashMap<NodeId" crates/forge_sema/src/
  should return zero production-code hits. Standard HashMap is ~3x slower
  than FxHashMap for u32 keys — if sema ends up over budget, this is the
  first place to check.

Targets (Ryzen 3600, Ubuntu 24.04):
  Test A — #include <stdio.h> + short main:
           release < 80ms, debug < 300ms
  Test B — 10 headers + 50-line program:
           release < 120ms, debug < 500ms

If substantially over: profile with cargo flamegraph, fix the hot path before
moving on. If substantially under: great, but don't over-optimize now.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 6 — Final report
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Produce phases/phase_04_report.md with:
  Code audit results
  Filled completeness matrix
  Stress test results
  Real-world test outcomes
  Performance numbers
  Total test count (forge_sema)
  Known deferrals (general constant expressions with address constants,
    exact long double semantics, _Complex, exact __int128, full K&R support,
    strict `inline` linkage semantics per C17 §6.7.4p7, `static` in array
    parameter brackets as an optimization hint, macro expansion backtraces
    in diagnostics — depends on Phase 2 SourceMap/ExpansionTable work which
    lands separately)
  Verdict: READY / NEEDS WORK

Final gate:
  cargo test --all              → green
  cargo clippy --all-targets --all-features -- -D warnings → clean
  cargo fmt --all -- --check    → clean

STOP. Do not commit. Hand the report and the above gate results to the user.
The user reviews and commits manually.
```

---

## Pitfalls & Debugging Tips

### "char + char = int, not char"
The #1 surprise. Both promote to int before the addition. Result is int. `char c = 'a' + 1;` involves: 'a'→int, add 1 (int), result int, implicit int→char for the assignment.

### "sizeof(\"hello\") is 6, not 8"
String literals are arrays, not pointers. They decay to pointers in most contexts, but `sizeof` is an exception. `sizeof("hello") = sizeof(char[6]) = 6`.

### "unsigned int + int → unsigned int, not int"
Same rank, unsigned wins. Subtle bug source: `if (unsigned_val - 1 < 0)` is always false because the subtraction is unsigned.

### "Shift operators don't do usual conversions"
Shifts promote each operand independently. Result type = promoted LEFT operand type. `(char)1 << 20` — char promotes to int, result is int. Right operand's type doesn't affect the result type.

### "Array parameters adjust to pointers"
`void f(int arr[10])` is `void f(int *arr)`. The 10 is discarded. `sizeof(arr)` inside is `sizeof(int *) = 8`, not 40.

### "Function declarations vs definitions"
`int f(int);` is a declaration. `int f(int x) { return x; }` is a definition. Multiple declarations OK if types match (merged by composite_type). Multiple definitions → error.

### "Empty parens != void parens"
`int f();` is unprototyped — it takes unspecified arguments (accepts anything, warns). `int f(void);` is a prototype with zero arguments. This matters for system headers.

### "Composite types matter"
`int f();` followed by `int f(int);` — compatible! The composite type is `int f(int)` and the second declaration is what calls check against. Messing this up breaks real-world headers.

### "Tag-based struct compatibility"
`struct A { int x; };` and `struct B { int x; };` are NOT compatible types, even if they have identical members. Only same-tag structs are compatible (within a TU).

### "Enum underlying type can widen"
`enum X { A = 3000000000 };` does NOT fit in signed int. The enum's underlying type widens to `unsigned int` (or wider). So `sizeof(enum X)` is still 4 on LP64 for `unsigned int`, but `sizeof(enum Y { Y = 5000000000 })` is 8. Individual enumerator constants still have type `int` for scope/promotion purposes — it's only the enum type itself that widens.

### "Array parameter qualifiers go on the POINTER, not the pointee"
`void f(int arr[const 10])` means `int *const arr` (pointer is const), NOT `const int *arr` (pointee is const). This catches people who read the syntax left-to-right. Same rule for `restrict`. The `static` keyword inside brackets is an assertion of minimum element count, not a qualifier.

### "Parameters and function body outer block are one scope"
`int f(int x) { int x = 5; }` is a redeclaration error — not "shadowing forbidden", but plain duplicate declaration. The two `x`s literally live in the same scope per C17 §6.2.1p4. Only `{ int x = 5; }` (one block deeper) is legal shadowing. Implementation trap: don't open a fresh block scope for the function body's top-level compound; reuse the function scope that already holds the parameters.

### "VLA sizeof has side effects"
`int n = 5; sizeof(int[n++]);` leaves `n` at 6. This is the one sizeof case where the operand is evaluated at runtime. Your sema representation must flag these so Phase 5 emits the evaluation code — a constant-folded sizeof here would be silently wrong.

### "Function pointers dereference forever without error"
`(*****f)()` is valid C. Each `*` on a function pointer yields a function designator, which decays back to a pointer. Don't write code that rejects `*` on pointer-to-function types — the "incomplete pointee" check in dereference applies to object types, not function types.

---

## Notes

- **Don't be perfect.** C has infinite obscure rules. Focus on what real programs need: integer promotions, pointer conversions, struct layout, function calls, assignment compatibility. VLA can be minimal. `_Complex` can be deferred.
- **Good diagnostics >> complete coverage.** "incompatible types: expected 'int *', found 'char'" with the right span is worth a hundred edge-case checks. This is why `QualType::to_c_string` is in Prompt 4.1.
- **Accumulate errors, don't abort.** One error should not hide all the others. Callers check `ctx.has_errors()` after analysis, not function return values.
- **The integer constant evaluator shares DNA** with the preprocessor's `#if` evaluator (Phase 2), but operates on AST nodes and handles `sizeof`, `_Alignof`, casts, and enum constants.
- **System header sema is the real test.** Everything else is preparation. If `#include <stdio.h>` plus a simple main doesn't sema-check cleanly, you are not done.
- **Deferrals for Phase 5 or later:** general constant expressions (address constants), full `__int128`, strict `_Complex`, K&R function definitions, `_Atomic` beyond qualifier parsing, complete bit-field packing edge cases.
