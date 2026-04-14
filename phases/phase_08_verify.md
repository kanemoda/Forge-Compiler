# Phase 8 — Verified Passes (Alive2-Style)

**Depends on:** Phase 6 (E-Graph Optimizer)
**Unlocks:** Phase 11 (Conformance — higher confidence)
**Estimated duration:** 12–20 days

---

## Goal

Build a verification system that can mechanically prove each e-graph rewrite rule preserves program semantics. By encoding source and target patterns as SMT formulas and asking Z3 to find counterexamples, we get mathematical confidence that our optimizations are correct. This is the trust/correctness differentiator.

---

## Deliverables

1. **`forge_verify` crate** — SMT-based translation validation for Forge IR rewrites
2. **IR → SMT encoding** — translate IR expressions into Z3 bitvector/array formulas
3. **Per-rule verification** — verify each e-graph rewrite rule in isolation
4. **Counterexample display** — when verification fails, show concrete inputs that demonstrate the bug
5. **Integration** — verification runs as a CI check on the rule set, not at compile time

---

## Technical Design

### Approach: Translation Validation per Rewrite Rule

For each rewrite rule `source_pattern => target_pattern`:
1. Introduce symbolic bitvector variables for each pattern variable (?x, ?y, etc.)
2. Encode the source pattern as an SMT expression
3. Encode the target pattern as an SMT expression
4. Assert that source ≠ target (the negation of equivalence)
5. Ask Z3 to find a satisfying assignment
6. If UNSAT: the rule is correct for all inputs
7. If SAT: the counterexample shows inputs where source ≠ target (the rule is buggy)

### SMT Encoding

Use the `z3` crate (Rust bindings to Z3 solver).

```rust
// Encoding (iadd ?x ?y) where ?x, ?y are 32-bit integers:
fn encode_iadd(ctx: &Context, x: &BV, y: &BV) -> BV {
    x.bvadd(y)  // Z3 bitvector addition
}

// Encoding (imul ?x ?y):
fn encode_imul(ctx: &Context, x: &BV, y: &BV) -> BV {
    x.bvmul(y)  // Z3 bitvector multiplication
}

// Encoding (shl ?x ?n):
fn encode_shl(ctx: &Context, x: &BV, n: &BV) -> BV {
    x.bvshl(n)  // Z3 bitvector shift left
}
```

### Verification of Example Rule

For rule: `(imul ?x 2) => (shl ?x 1)`

```rust
fn verify_mul_to_shl(ctx: &Context) -> VerifyResult {
    let x = BV::new_const(ctx, "x", 32);
    let two = BV::from_i64(ctx, 2, 32);
    let one = BV::from_i64(ctx, 1, 32);
    
    let source = x.bvmul(&two);       // x * 2
    let target = x.bvshl(&one);       // x << 1
    
    let solver = Solver::new(ctx);
    solver.assert(&source._eq(&target).not());  // assert they differ
    
    match solver.check() {
        SatResult::Unsat => VerifyResult::Proven,  // no counterexample exists!
        SatResult::Sat => {
            let model = solver.get_model().unwrap();
            let x_val = model.eval(&x, true).unwrap();
            VerifyResult::Counterexample { x: x_val.to_string() }
        }
        SatResult::Unknown => VerifyResult::Timeout,
    }
}
```

### What Can Be Verified

**Verifiable (pure operations):**
- All algebraic simplification rules
- Strength reduction rules
- Constant folding correctness
- Comparison simplifications
- Bitwise operation identities

**Not easily verifiable (side effects):**
- Memory-related optimizations (load/store reordering) — would need a memory model
- Control flow transformations — would need a more complex encoding

Start with pure operation rules (which covers most of the e-graph rewrites).

---

## Acceptance Criteria

- [ ] All algebraic identity rules verified (x+0=x, x*1=x, x-x=0, etc.)
- [ ] Commutativity rules verified for all widths (8, 16, 32, 64 bit)
- [ ] Strength reduction rules verified (mul by power of 2 → shift)
- [ ] At least one deliberate bug detected: introduce a wrong rule, verify the checker catches it
- [ ] Counterexample display shows concrete values
- [ ] Verification of all rules completes in under 60 seconds in CI
- [ ] CI job runs verification on every PR touching rewrite rules

---

## Claude Code Prompts

### Prompt 8.1 — Z3 integration and basic SMT encoding

```
Create the forge_verify crate in the Forge workspace. Add the `z3` crate as a dependency (use z3 = "0.12" or latest).

Note: Z3 requires the Z3 solver to be installed on the system. Add a note to CLAUDE.md about this: `sudo apt install libz3-dev` on Ubuntu.

Implement the SMT encoding of Forge IR operations in forge_verify/src/encode.rs:

1. A function to create symbolic bitvector variables for a given bit width
2. Encoding functions for each pure IR operation:
   - iadd(x, y) → x.bvadd(y)
   - isub(x, y) → x.bvsub(y)
   - imul(x, y) → x.bvmul(y)
   - idiv(x, y) → x.bvsdiv(y) (note: signed division)
   - udiv(x, y) → x.bvudiv(y)
   - irem(x, y) → x.bvsrem(y)
   - urem(x, y) → x.bvurem(y)
   - band(x, y) → x.bvand(y)
   - bor(x, y) → x.bvor(y)
   - bxor(x, y) → x.bvxor(y)
   - bnot(x) → x.bvnot()
   - shl(x, y) → x.bvshl(y)
   - sshr(x, y) → x.bvashr(y)
   - ushr(x, y) → x.bvlshr(y)
   - sext(x, target_width) → x.sign_ext(target_width - source_width)
   - zext(x, target_width) → x.zero_ext(target_width - source_width)
   - trunc(x, target_width) → x.extract(target_width - 1, 0)
   - icmp_eq(x, y) → x._eq(y)
   - icmp_slt(x, y) → x.bvslt(y)
   - etc. for all comparisons

3. A VerifyResult enum: Proven, Counterexample { values: HashMap<String, String> }, Timeout, Error(String)

Write basic tests:
- Verify that iadd is commutative: prove x+y == y+x for 32-bit
- Verify that imul is NOT the same as iadd: find counterexample
```

### Prompt 8.2 — Rewrite rule verification engine

```
Implement a system that can verify e-graph rewrite rules in forge_verify/src/verify_rules.rs.

1. Define a RewritePattern type that mirrors the egg rewrite patterns:
   - Pattern nodes: Op(opcode, children), Var(name), Const(value)
   - This should be constructable from egg's Pattern type or from a string representation

2. encode_pattern(ctx: &Context, pattern: &RewritePattern, vars: &HashMap<String, BV>, width: u32) -> BV
   - Recursively encode a pattern into a Z3 bitvector expression
   - Variables (?x, ?y) map to symbolic BV variables
   - Constants map to BV::from_i64

3. verify_rule(source: &RewritePattern, target: &RewritePattern, bit_widths: &[u32]) -> VerifyResult
   - For each bit width (8, 16, 32, 64):
     - Create symbolic variables for all pattern variables at that width
     - Encode source and target patterns
     - Assert source != target
     - Check satisfiability
   - Return Proven only if UNSAT for ALL bit widths
   - If any width produces a counterexample, return it with the specific width and values

4. verify_all_rules(rules: &[NamedRule]) -> Vec<(String, VerifyResult)>
   - Run verification for every rule in the optimizer's rule set
   - Return results for each rule by name

Write tests:
- Verify "add-zero": (iadd ?x 0) => ?x — should be Proven
- Verify "mul-one": (imul ?x 1) => ?x — should be Proven
- Verify "sub-self": (isub ?x ?x) => 0 — should be Proven
- Verify a WRONG rule: (iadd ?x 1) => ?x — should find counterexample
- Verify "mul-pow2": (imul ?x 2) => (shl ?x 1) — should be Proven
- Verify strength reduction with division: (idiv ?x 2) => (sshr ?x 1) — this should FAIL for negative odd numbers! (shows the value of verification)
```

### Prompt 8.3 — Counterexample display and CI integration

```
Improve the verification output and integrate it into the project.

1. Pretty counterexample display:
   When verification fails, print:
   ```
   FAILED: rule "div-pow2" at 32-bit
   Counterexample:
     ?x = -3 (0xFFFFFFFD)
   Source: (idiv ?x 2) = -1 (0xFFFFFFFF)
   Target: (sshr ?x 1) = -2 (0xFFFFFFFE)
   ```

2. Create a binary/test that verifies all optimizer rules:
   - forge_verify/src/bin/verify_rules.rs (or an integration test)
   - Imports the rule set from forge_egraph
   - Runs verify_all_rules
   - Prints a summary: "42/44 rules verified, 2 FAILED"
   - Exit code 1 if any rule fails

3. Add a GitHub Actions CI step that:
   - Installs Z3 (apt install libz3-dev)
   - Runs the rule verifier
   - Fails the build if any rule is not verified

4. Fix any rules that fail verification:
   - The signed division → shift rule needs a precondition (x >= 0) or should be removed
   - Add the precondition mechanism: some rules only apply when certain conditions hold

5. Add an `--experimental` flag for unverified rules — rules that haven't been proven correct are only enabled with this flag.

Write documentation: a doc comment in forge_egraph explaining which rules are verified and what that means.
```

### Prompt 8.4 — Verification of conditional rules and edge cases

```
Extend the verifier to handle more complex rule patterns.

1. Conditional rules: rules that only apply under certain conditions
   - Example: (udiv ?x ?n) => (ushr ?x log2(?n)) WHEN ?n is a power of 2
   - Encode the condition as a premise: solver.assert(&is_power_of_2(n))
   - Then check source == target under that assumption

2. Multi-width rules: some rules behave differently at different widths
   - Verify each rule at 8, 16, 32, and 64-bit widths independently
   - Flag rules that only hold at specific widths

3. Undefined behavior cases:
   - Division by zero is UB in C — we can exclude it: solver.assert(&y._eq(&zero).not())
   - Shift by >= bit width is UB — exclude it
   - Signed overflow is UB — this is tricky, as wrapping behavior in the IR may differ from C semantics

4. Floating-point rules:
   - Z3 supports IEEE 754 floating-point theory
   - Verify float rules where applicable (be careful: floating-point is not associative!)
   - fadd(x, 0.0) => x — only true if x is not -0.0 and there are no NaN issues

5. Write a comprehensive test suite verifying every rule in the optimizer. This becomes a regression test: if anyone adds a new rule, they must also add verification.
```

---

## Notes

- Z3 is a heavy dependency (~50MB). It's only needed for development/CI, not for running the compiler. Make it an optional feature: `cargo test --features verify`.
- Verification time per rule is typically <1 second for bitvector proofs at 32/64 bits. The whole rule set should verify in well under a minute.
- This is one of Forge's biggest selling points. Very few compilers mechanically verify their optimizations. LLVM's Alive2 project does this, but it's external tooling — Forge has it built in.
- Start simple: verify the pure arithmetic rules first. Memory and control flow verification can come later.
