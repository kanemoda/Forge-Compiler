# Phase 6 — E-Graph Optimizer

**Depends on:** Phase 5 (IR)
**Unlocks:** Phase 8 (Verified Passes)
**Estimated duration:** 14–25 days

---

## Goal

Build the core technical differentiator of Forge: an optimization pass based on equality saturation using e-graphs. Instead of applying optimizations in a fixed order (where early decisions can block later ones), the e-graph explores the entire space of equivalent programs simultaneously, then extracts the optimal version using a cost function.

---

## Deliverables

1. **`forge_egraph` crate** — integration with the `egg` crate for equality saturation
2. **Forge IR → e-graph encoding** — convert IR instructions into e-graph e-nodes
3. **Rewrite rules** — algebraic simplification, constant folding, strength reduction, CSE, and more
4. **Cost function** — extract the cheapest equivalent program from the saturated e-graph
5. **E-graph → Forge IR extraction** — convert back from the optimized e-graph to valid IR
6. **Pipeline integration** — the optimizer runs between IR lowering and codegen

---

## Background: Why E-Graphs?

Traditional compilers apply optimizations as a sequence of passes. The order matters: constant propagation before dead code elimination, inlining before constant propagation, etc. Getting the order right is a black art, and a poor order can miss optimization opportunities.

E-graphs (equality graphs) represent *sets of equivalent programs* in a compact data structure. Equality saturation applies ALL rewrite rules simultaneously until no more rewrites apply (saturation), then extracts the best program according to a cost function. This eliminates phase-ordering problems entirely.

The `egg` library (Rust) is the reference implementation. It provides the core e-graph data structure, pattern matching, and extraction — we supply the language definition and rewrite rules.

---

## Technical Design

### Language Definition for egg

Define the "language" of Forge IR as an egg Language:

```rust
use egg::{define_language, Id, Symbol};

define_language! {
    pub enum ForgeIR {
        // Constants
        "iconst" = Iconst(i64),
        "fconst" = Fconst([Id; 0]),  // f64 stored separately
        
        // Arithmetic
        "iadd" = Iadd([Id; 2]),
        "isub" = Isub([Id; 2]),
        "imul" = Imul([Id; 2]),
        "idiv" = Idiv([Id; 2]),
        "urem" = Urem([Id; 2]),
        
        // Bitwise
        "band" = Band([Id; 2]),
        "bor"  = Bor([Id; 2]),
        "bxor" = Bxor([Id; 2]),
        "shl"  = Shl([Id; 2]),
        "sshr" = Sshr([Id; 2]),
        
        // Comparison
        "icmp_eq" = IcmpEq([Id; 2]),
        "icmp_ne" = IcmpNe([Id; 2]),
        // ... other comparisons
        
        // Conversions
        "sext" = Sext([Id; 1]),
        "zext" = Zext([Id; 1]),
        "trunc" = Trunc([Id; 1]),
        
        // Memory (treated carefully — not freely reorderable)
        "load" = Load([Id; 1]),
        "store" = Store([Id; 2]),
        
        // Control
        "select" = Select([Id; 3]),
        
        // Variables / block params
        Symbol(Symbol),
    }
}
```

### Rewrite Rules (Starter Set)

```rust
use egg::rewrite;

fn rules() -> Vec<Rewrite<ForgeIR, ()>> {
    vec![
        // ========== Algebraic Identities ==========
        rewrite!("add-zero";  "(iadd ?x 0)" => "?x"),
        rewrite!("mul-one";   "(imul ?x 1)" => "?x"),
        rewrite!("mul-zero";  "(imul ?x 0)" => "0"),
        rewrite!("sub-self";  "(isub ?x ?x)" => "0"),
        rewrite!("xor-self";  "(bxor ?x ?x)" => "0"),
        rewrite!("and-self";  "(band ?x ?x)" => "?x"),
        rewrite!("or-self";   "(bor ?x ?x)" => "?x"),
        
        // ========== Commutativity ==========
        rewrite!("add-comm";  "(iadd ?x ?y)" => "(iadd ?y ?x)"),
        rewrite!("mul-comm";  "(imul ?x ?y)" => "(imul ?y ?x)"),
        rewrite!("and-comm";  "(band ?x ?y)" => "(band ?y ?x)"),
        rewrite!("or-comm";   "(bor ?x ?y)" => "(bor ?y ?x)"),
        
        // ========== Associativity ==========
        rewrite!("add-assoc"; "(iadd (iadd ?x ?y) ?z)" => "(iadd ?x (iadd ?y ?z))"),
        rewrite!("mul-assoc"; "(imul (imul ?x ?y) ?z)" => "(imul ?x (imul ?y ?z))"),
        
        // ========== Strength Reduction ==========
        rewrite!("mul-pow2";  "(imul ?x 2)" => "(shl ?x 1)"),
        rewrite!("mul-pow2-4"; "(imul ?x 4)" => "(shl ?x 2)"),
        rewrite!("mul-pow2-8"; "(imul ?x 8)" => "(shl ?x 3)"),
        rewrite!("div-pow2";  "(idiv ?x 2)" => "(sshr ?x 1)"),  // signed only when non-negative
        
        // ========== Constant Folding ==========
        // Implemented via egg Analysis, not rewrites
        
        // ========== Distributivity ==========
        rewrite!("dist-mul-add"; "(imul ?x (iadd ?y ?z))" => "(iadd (imul ?x ?y) (imul ?x ?z))"),
        
        // ========== Boolean/Comparison ==========
        rewrite!("not-not";      "(bxor (bxor ?x -1) -1)" => "?x"),
        rewrite!("select-true";  "(select 1 ?x ?y)" => "?x"),
        rewrite!("select-false"; "(select 0 ?x ?y)" => "?y"),
        rewrite!("select-same";  "(select ?c ?x ?x)" => "?x"),
    ]
}
```

### Constant Folding via egg Analysis

Use `egg::Analysis` to propagate constant values:

```rust
struct ConstFold;
impl Analysis<ForgeIR> for ConstFold {
    type Data = Option<i64>;  // known constant value, if any
    
    fn make(egraph: &EGraph<ForgeIR, Self>, enode: &ForgeIR) -> Self::Data {
        match enode {
            ForgeIR::Iconst(n) => Some(*n),
            ForgeIR::Iadd([a, b]) => {
                let a = egraph[*a].data?;
                let b = egraph[*b].data?;
                Some(a.wrapping_add(b))
            }
            // ... other ops
            _ => None,
        }
    }
    fn merge(&mut self, a: &mut Self::Data, b: Self::Data) -> DidMerge { ... }
}
```

### Cost Function

```rust
struct ForgeCost;
impl egg::CostFunction<ForgeIR> for ForgeCost {
    type Cost = u64;
    fn cost<C>(&mut self, enode: &ForgeIR, mut costs: C) -> u64
    where C: FnMut(Id) -> u64 {
        let op_cost = match enode {
            ForgeIR::Iconst(_) => 0,     // free
            ForgeIR::Iadd(_) => 1,       // cheap
            ForgeIR::Imul(_) => 3,       // moderate
            ForgeIR::Idiv(_) => 20,      // expensive
            ForgeIR::Shl(_) => 1,        // cheap
            ForgeIR::Load(_) => 4,       // memory access
            _ => 1,
        };
        enode.fold(op_cost, |sum, id| sum + costs(id))
    }
}
```

---

## Acceptance Criteria

- [ ] Forge IR instructions encode into e-graph e-nodes
- [ ] Algebraic simplification rules fire correctly (x + 0 → x, x * 1 → x, etc.)
- [ ] Constant folding evaluates known constants at compile time
- [ ] Strength reduction replaces mul/div by powers of 2 with shifts
- [ ] Common subexpressions are automatically unified (CSE is inherent in e-graphs!)
- [ ] The cost function extracts reasonable code (prefers shifts over divides, etc.)
- [ ] E-graph → IR extraction produces valid IR (passes verifier)
- [ ] Optimizing a function with redundant computation produces measurably smaller IR
- [ ] The optimizer handles functions with 100+ instructions without timing out

---

## Claude Code Prompts

### Prompt 6.1 — egg integration and language definition

```
Create the forge_egraph crate in the Forge workspace. Add `egg` as a dependency.

Define the e-graph language for Forge IR using egg's define_language! macro. Cover these operations:
- Integer constants (store the i64 directly)
- Integer arithmetic: iadd, isub, imul, idiv, udiv, irem, urem
- Float arithmetic: fadd, fsub, fmul, fdiv
- Bitwise: band, bor, bxor, bnot, shl, sshr, ushr
- Comparison: icmp_eq, icmp_ne, icmp_slt, icmp_sle, icmp_sgt, icmp_sge, icmp_ult, icmp_ule, icmp_ugt, icmp_uge
- Conversions: sext, zext, trunc
- Select (ternary)
- Variable references as Symbols

Implement the constant folding Analysis:
- Track Option<i64> for each e-class
- Fold iadd, isub, imul when both operands are known constants
- When a constant is discovered, add the constant node to the e-class

Write tests:
- Create an e-graph with (iadd 3 4), run constant folding, verify the e-class contains 7
- Create (imul (iadd 1 2) (isub 5 3)), verify it folds to 6
```

### Prompt 6.2 — Rewrite rules

```
Implement the rewrite rule set for forge_egraph.

Create a function pub fn optimization_rules() -> Vec<Rewrite<ForgeIR, ConstFold>> that returns all rewrite rules.

Categories:

1. Algebraic identities (both directions where applicable):
   - x + 0 → x, x - 0 → x, x * 1 → x, x * 0 → 0
   - x - x → 0, x ^ x → 0, x & x → x, x | x → x
   - x & 0 → 0, x | -1 → -1 (all ones)
   - x + (-x) → 0

2. Commutativity: add, mul, and, or, xor, eq, ne

3. Associativity: add, mul, and, or, xor (both directions so the e-graph can explore all groupings)

4. Strength reduction:
   - x * 2 → x << 1, x * 4 → x << 2, x * 8 → x << 3 (generalize for all powers of 2 using a conditional applier)
   - x / 2 → x >> 1 (for unsigned only — need a way to track signedness)

5. Distributivity: x * (y + z) ↔ x*y + x*z (both directions — let cost function decide which is better)

6. Boolean/select simplification:
   - select(true, a, b) → a, select(false, a, b) → b
   - select(c, a, a) → a
   - !!x → x (double negation)

7. Comparison simplifications:
   - (x == x) → true, (x != x) → false
   - (x < x) → false

Write comprehensive tests: for each rule category, construct an expression, run the optimizer (Runner with the rules for a bounded number of iterations), extract the result, and verify it's simplified.
```

### Prompt 6.3 — IR to e-graph encoding and extraction

```
Implement the conversion between Forge IR and the e-graph representation.

forge_egraph/src/encode.rs — IR → e-graph:
1. Take a forge_ir::Function as input
2. Walk each basic block's instructions in order
3. For each instruction, create the corresponding e-node and add it to the e-graph
4. Map SSA values (forge_ir::Value) to e-graph Ids
5. Handle block parameters: represent them as Symbol nodes with unique names
6. Handle memory operations carefully: loads and stores have side effects and cannot be freely reordered. For now, keep them outside the e-graph (don't optimize them) — only optimize pure arithmetic/logic subexpressions.
7. Return the e-graph and a mapping from Value → Id and Block → list of root Ids

forge_egraph/src/extract.rs — e-graph → IR:
1. After running the optimizer, use the cost function to extract the best expression for each root
2. Convert the extracted e-nodes back into forge_ir instructions
3. Rebuild the function with the same block structure but optimized instruction sequences
4. Run the IR verifier on the result

Implement an optimize_function(func: &Function) -> Function pipeline that:
1. Encodes the function into an e-graph
2. Adds all rewrite rules
3. Runs the egg Runner for a configurable iteration limit (default: 10 iterations or 10 seconds)
4. Extracts the optimized program using the cost function
5. Returns a new, optimized Function

Write tests:
- A function with `x * 2` optimized to `x << 1`
- A function with `(a + b) + (a + b)` — CSE should share the subexpression
- A function with constant expressions folded away
- A function with no optimization opportunities — output should be equivalent to input
```

### Prompt 6.4 — Advanced rules and cost tuning

```
Extend forge_egraph with more sophisticated optimizations and a tuned cost function.

Additional rules:

1. Dead code style: if a value is only used as an argument to an operation that simplifies it away, the e-graph naturally handles this — but add rules for:
   - Redundant loads: if we store X to P then load from P (with no intervening store), the load result equals X
   - This requires memory-aware analysis — implement a simple "last store" tracking

2. Conditional rules (use egg ConditionalApplier):
   - Multiplication by power of 2: `(imul ?x ?n)` => `(shl ?x log2(?n))` when ?n is a known constant power of 2
   - Division by power of 2 for unsigned: similarly

3. Canonicalization rules (one-directional):
   - Subtraction of constant → addition of negated constant: (isub ?x C) → (iadd ?x -C)
   - This normalizes the representation so other rules can match

Tuned cost function:
- iconst: 0 (free, encoded as immediate)
- iadd, isub, band, bor, bxor: 1
- shl, sshr, ushr: 1
- imul: 3 (multi-cycle on most architectures)
- idiv, urem, irem: 25 (very expensive)
- load: 4 (memory latency)
- select: 2
- sext, zext, trunc: 0 (often free in hardware, just reinterpretation)
- Tie-breaking: prefer fewer total nodes (smaller code size)

Write tests:
- x * 7 does NOT get strength-reduced (7 is not a power of 2)
- x * 8 DOES get reduced to x << 3
- A complex expression like (a*2 + b*2) gets optimized to (a+b) << 1 via distributivity + strength reduction
- Benchmark: optimize a function with 200 arithmetic instructions, verify it completes in under 5 seconds
```

### Prompt 6.5 — Pipeline integration

```
Integrate the e-graph optimizer into the Forge compilation pipeline.

1. Update forge_driver to run the optimizer after IR lowering:
   Pipeline: Source → Lex → Preprocess → Parse → Sema → IR Lowering → E-Graph Optimize → (future: Codegen)

2. Add CLI flags:
   - -O0: no optimization (skip e-graph)
   - -O1: basic optimizations (limited iterations)
   - -O2: full optimization (default iteration limit)
   - --print-opt-stats: show how many rewrites were applied, e-classes created, time spent

3. Add forge emit-ir-opt <file.c> to show IR after optimization.

4. Create comparison tests in tests/lit/egraph/:
   - Write C functions with known optimization opportunities
   - Verify the optimized IR is simpler than the unoptimized IR
   - tests/lit/egraph/constant_fold.c — 2 + 3 should be 5 in the IR
   - tests/lit/egraph/strength_reduce.c — x * 4 should become x << 2
   - tests/lit/egraph/cse.c — same subexpression used twice should be computed once

5. All existing tests must still pass after adding the optimizer.

6. Cargo clippy, all tests green.
```

---

## Notes

- **Memory operations are the hard part.** Pure arithmetic is easy to reason about in e-graphs. Memory operations (load, store) have side effects and ordering constraints. For this phase, keep memory operations outside the e-graph and only optimize pure computation. Phase 8 (verification) will help ensure correctness for more aggressive memory-related optimizations later.
- **Saturation can blow up.** Some rule sets cause exponential e-graph growth. Use egg's iteration and node limits. Start conservative (5 iterations, 10K nodes), increase as confidence grows.
- **The cost function is everything.** A bad cost function can extract worse code than the original. When in doubt, prefer fewer operations and smaller code size.
- CSE (common subexpression elimination) is *free* in e-graphs — it's an inherent property of the data structure. This alone makes the approach worthwhile.
