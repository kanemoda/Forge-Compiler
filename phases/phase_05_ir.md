# Phase 5 — Forge IR

**Depends on:** Phase 4 (Semantic Analysis)
**Unlocks:** Phase 6 (E-Graph), Phase 7 (Codegen), Phase 9 (Incremental)
**Estimated duration:** 7–12 days

---

## Goal

Design and implement Forge's intermediate representation — a typed, SSA-based IR that sits between the typed AST and machine code. The IR must be designed from day one to feed naturally into the e-graph optimizer and be serializable for incremental compilation caching.

---

## Deliverables

1. **`forge_ir` crate** — IR types, builder, printer, parser, and verifier
2. **SSA construction** — AST-to-IR lowering with automatic SSA via the "sealed blocks" algorithm
3. **IR printer** — human-readable text format (like LLVM IR but simpler)
4. **IR parser** — parse the text format back into IR (for testing)
5. **IR verifier** — checks SSA dominance, type consistency, and well-formedness

---

## Technical Design

### IR Structure

```
Module
  └─ Function*
       ├─ name, return_type, params
       └─ Block* (basic blocks)
            ├─ label
            ├─ params (block parameters instead of phi nodes — like MLIR/Cranelift)
            └─ Inst* (instructions)
                 ├─ result: Value
                 ├─ opcode: Opcode
                 └─ operands: Vec<Value>
```

**Block parameters instead of phi nodes:** Instead of traditional φ-nodes, use block parameters (like Cranelift and MLIR). When a block has multiple predecessors, each predecessor passes values explicitly as block arguments. This is cleaner for e-graph extraction and optimization.

### Opcodes

```rust
pub enum Opcode {
    // Constants
    Iconst(i64),       // integer constant
    Fconst(f64),       // float constant
    
    // Arithmetic (integer)
    Iadd, Isub, Imul, Idiv, Udiv, Irem, Urem,
    // Arithmetic (float)
    Fadd, Fsub, Fmul, Fdiv, Frem,
    // Bitwise
    Band, Bor, Bxor, Bnot, Shl, Sshr, Ushr,
    // Comparison
    Icmp(CmpOp),  // eq, ne, lt, le, gt, ge (signed and unsigned variants)
    Fcmp(CmpOp),
    // Conversion
    Sext, Zext, Trunc, Fpext, Fptrunc,
    Sitofp, Uitofp, Fptosi, Fptoui,
    Bitcast, Inttoptr, Ptrtoint,
    // Memory
    Load, Store,
    StackAlloc,       // allocate stack slot, returns pointer
    GetElementPtr,    // pointer arithmetic (struct field, array index)
    // Control flow
    Jump,             // unconditional branch to block with args
    Branch,           // conditional branch: if cond then blockA else blockB
    Return,
    Call,
    // Misc
    Select,           // ternary: select cond, true_val, false_val
    Phi,              // only if we fallback from block params
}
```

### Value and Type

```rust
pub struct Value(u32);  // SSA value handle

pub enum IrType {
    I1, I8, I16, I32, I64,    // integer types
    F32, F64,                  // float types
    Ptr,                       // opaque pointer (like LLVM's ptr)
    Void,
    Aggregate(AggregateId),    // structs, arrays
}
```

### IR Text Format

```
func @main() -> i32 {
  entry():
    %0 = iconst.i32 42
    %1 = iconst.i32 1
    %2 = iadd.i32 %0, %1
    return %2
    
  ; function with control flow
func @max(i32, i32) -> i32 {
  entry(%a: i32, %b: i32):
    %cmp = icmp.gt %a, %b
    branch %cmp, then(%a), else(%b)
  then(%result: i32):
    return %result
  else(%result: i32):
    return %result
}
```

### AST-to-IR Lowering

Use the "sealed blocks" algorithm for SSA construction:
1. Create basic blocks for control flow (if/else branches, loop headers, etc.)
2. Use a variable map per block: when a variable is assigned, record the value
3. When a variable is read, look up the current block's map, or recursively search predecessor blocks
4. When a block is "sealed" (all predecessors known), insert block parameters to merge values from different paths

---

## Acceptance Criteria

- [ ] All C17 expression types lower to IR instructions
- [ ] Control flow (if/else, loops, switch) correctly lowers to basic blocks with branches
- [ ] Structs and arrays lower to stack allocations with GEP (get element pointer)
- [ ] Function calls lower to `call` instructions with proper ABI types
- [ ] Global variables lower to module-level symbols
- [ ] The IR printer produces human-readable output
- [ ] The IR parser can round-trip: print → parse → print yields the same text
- [ ] The IR verifier catches: type mismatches, use-before-def, unreachable blocks, missing terminators
- [ ] Can lower a 50-line C function to correct IR

---

## Claude Code Prompts

### Prompt 5.1 — IR types and data structures

```
Create the forge_ir crate in the Forge workspace. Define the core IR data structures.

Implement in forge_ir/src/lib.rs (split into modules as appropriate):

1. Value — a newtype wrapper around u32, representing an SSA value
2. Block — a newtype wrapper around u32, representing a basic block
3. IrType — enum: I1, I8, I16, I32, I64, F32, F64, Ptr, Void, Aggregate(u32)
4. CmpOp — enum: Eq, Ne, Slt, Sle, Sgt, Sge, Ult, Ule, Ugt, Uge
5. Opcode — enum covering all operations (see the phase doc for full list)

6. Instruction struct:
   - result: Option<Value> (None for store, branch, return)
   - opcode: Opcode
   - ty: IrType (result type)
   - operands: Vec<Operand>
   - span: Option<Span> (for debug info back to source)

7. Operand enum: Value(Value), Block(Block), IntConst(i64), FloatConst(f64), Type(IrType)

8. BasicBlock struct:
   - id: Block
   - params: Vec<(Value, IrType)>  — block parameters (SSA merge points)
   - instructions: Vec<Instruction>
   - (the last instruction must be a terminator: Jump, Branch, Return)

9. Function struct:
   - name: String
   - params: Vec<(Value, IrType)>
   - return_type: IrType
   - blocks: Vec<BasicBlock>
   - entry: Block

10. Module struct:
    - functions: Vec<Function>
    - globals: Vec<Global>  (global variables)

Implement Display for all types so they print in the text format described in the phase doc.
Write tests constructing a simple function (main returning 42) in IR and printing it.
```

### Prompt 5.2 — IR builder

```
Implement an IR builder API in forge_ir/src/builder.rs.

The builder provides a convenient API for constructing IR, used by the AST lowering pass.

FunctionBuilder:
- new(name, params, return_type) -> FunctionBuilder
- create_block() -> Block
- switch_to_block(block) — set the current insertion point
- seal_block(block) — mark that all predecessors of this block are known
- append_block_param(block, ty) -> Value
- current_block() -> Block

Instruction insertion (all return Value):
- iconst(ty, value) -> Value
- fconst(ty, value) -> Value
- iadd(lhs, rhs) -> Value
- isub, imul, idiv, udiv, irem, urem — same pattern
- fadd, fsub, fmul, fdiv — same
- band, bor, bxor, bnot, shl, sshr, ushr
- icmp(op, lhs, rhs) -> Value (returns I1)
- fcmp(op, lhs, rhs) -> Value
- sext(val, target_ty), zext, trunc, fpext, fptrunc, sitofp, uitofp, fptosi, fptoui, bitcast
- load(ptr, ty) -> Value
- store(ptr, val) — no return value
- stack_alloc(ty) -> Value (returns Ptr)
- gep(base_ptr, offsets, result_ty) -> Value
- select(cond, true_val, false_val) -> Value
- call(func_name, args, return_ty) -> Value

Terminators (end a basic block):
- jump(target_block, args)
- branch(cond, true_block, true_args, false_block, false_args)
- return_val(val) / return_void()

SSA variable tracking:
- declare_var(name) -> VarId — declare a local variable for SSA construction
- def_var(var, value) — assign to a variable in the current block
- use_var(var, ty) -> Value — read a variable, automatically inserting block params if needed

Build the same "main returns 42" function using the builder API to verify it works.
Build a function with an if/else to test block parameters.
```

### Prompt 5.3 — AST-to-IR lowering

```
Implement AST-to-IR lowering in forge_ir/src/lower.rs (or a separate forge_lower crate if preferred).

This translates the typed AST from forge_sema into Forge IR.

Lower each construct:

Expressions:
- Integer/float literals → iconst/fconst
- Variable reference → use_var (looks up the SSA value)
- Binary arithmetic → iadd/isub/imul/etc. (with appropriate type)
- Comparisons → icmp/fcmp
- Logical && and || → short-circuit evaluation using branches
- Assignment → def_var the new value (and store if the variable is a pointer/memory location)
- Function call → call instruction
- Array subscript → gep + load
- Struct member access → gep with field offset + load
- Address-of (&x) → return the pointer (stack_alloc for locals)
- Dereference (*p) → load
- Cast → appropriate conversion instruction (sext, zext, trunc, sitofp, etc.)
- Sizeof → iconst with the computed size
- Ternary → branch + block params or select

Statements:
- Variable declaration → stack_alloc + optional store for initializer
- Expression statement → evaluate expression, discard result
- Return → return instruction
- If/else → create then/else/merge blocks, branch on condition
- While → create header/body/exit blocks. Header: evaluate condition, branch. Body: evaluate, jump back to header.
- Do-while → body block (entered unconditionally first time), then condition check
- For → init, then loop header/body/update/exit structure
- Switch → chain of comparisons and branches (or jump table optimization later)
- Break → jump to loop/switch exit block
- Continue → jump to loop update/header block
- Goto/Label → blocks for labels, jump instructions for goto

Functions:
- Create entry block with parameters
- Lower the body
- Handle functions that don't explicitly return (add implicit return for void functions)

Globals:
- Global variables → module-level Global entries with initializer data

Write tests:
- Lower a function computing factorial iteratively
- Lower a function with if/else control flow
- Lower a function with a struct parameter
- Verify the IR output is valid (run the verifier from prompt 5.4)
```

### Prompt 5.4 — IR verifier and parser

```
Implement an IR verifier and a text-format parser for forge_ir.

IR Verifier (forge_ir/src/verify.rs):
Check these invariants:
1. Every block ends with exactly one terminator instruction (Jump, Branch, Return)
2. No instructions after a terminator
3. Every Value used in an instruction is defined before its use (SSA dominance)
4. Type consistency: operand types match what the opcode expects
5. Branch targets exist in the function
6. Block parameter counts match what callers pass via Jump/Branch args
7. Function return type matches Return instruction types
8. Entry block has no predecessors passing block params
9. No unreachable blocks (warning, not error)

IR Parser (forge_ir/src/parse.rs):
Parse the text format back into Module/Function/Block/Instruction structures.
This is primarily for testing: we can write IR tests in text form.

Grammar:
  module := function*
  function := 'func' '@' name '(' param_list ')' '->' type '{' block* '}'
  block := label '(' param_list ')' ':' instruction*
  instruction := '%' name '=' opcode '.' type operand (',' operand)* | opcode operand*
  operand := '%' name | block_ref | integer | float

Implement round-trip test: construct IR → print → parse → print → assert equal.

Write tests:
- Verifier catches: use before def, type mismatch, missing terminator, wrong block param count
- Parser handles: simple function, function with control flow, multiple functions
- Round-trip test passes
```

### Prompt 5.5 — Wire IR into driver

```
Integrate the IR into the Forge driver pipeline.

1. Update forge_driver to:
   - After sema, lower the typed AST to Forge IR
   - forge check now shows IR output (add --emit-ir flag)
   - Add forge emit-ir <file.c> subcommand

2. Create lit tests in tests/lit/ir/:
   - tests/lit/ir/arithmetic.c — basic arithmetic lowers correctly
   - tests/lit/ir/control_flow.c — if/else/while/for produce correct blocks
   - tests/lit/ir/functions.c — function calls, parameters, return values
   - tests/lit/ir/structs.c — struct access lowers to GEP
   - tests/lit/ir/globals.c — global variable lowering

3. Write a factorial function in C, compile to IR, manually verify the IR is correct.

4. Run the IR verifier on all generated IR as part of tests.

5. Cargo clippy, all tests pass.
```

---

## Notes

- The IR design should be kept simple. We're not building LLVM. Start with ~30 opcodes covering basic operations, memory, control flow, and conversions. Add more as needed.
- Block parameters (instead of phi nodes) are a key design choice — they make e-graph integration cleaner because phi semantics are explicit in the block edge arguments.
- Consider using an arena or "secondary map" data structure (like `cranelift_entity::EntityList`) for efficient Value/Block storage.
- The IR verifier is not optional. It should run after every transformation in debug builds.
