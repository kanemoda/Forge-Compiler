# Phase 7 — Code Generation (x86-64 + AArch64)

**Depends on:** Phase 5 (IR), Phase 6 (E-Graph — optional, codegen works on unoptimized IR too)
**Unlocks:** Phase 8 (Verify), Phase 10 (Energy), Phase 11 (Conformance)
**Estimated duration:** 18–30 days

---

## Goal

Translate Forge IR into machine code for x86-64 and AArch64, emit ELF object files, and invoke a system linker to produce executables. After this phase, Forge can compile C programs into running binaries.

---

## Deliverables

1. **`forge_codegen` crate** — shared codegen infrastructure: register allocation, stack layout, calling conventions
2. **`forge_x86_64` crate** — x86-64 instruction selection, encoding, ELF emission
3. **`forge_aarch64` crate** — AArch64 instruction selection, encoding, ELF emission
4. **Linker invocation** — call system `ld` or `cc` to link object files into executables
5. **End-to-end test** — compile and run "Hello, World!" and simple C programs

---

## Technical Design

### Codegen Pipeline

```
Forge IR (optimized)
    │
    ▼
Instruction Selection (IR → MachineInst)
    │
    ▼
Register Allocation (virtual regs → physical regs)
    │
    ▼
Prologue/Epilogue Insertion (stack frame setup)
    │
    ▼
Machine Code Encoding (MachineInst → bytes)
    │
    ▼
ELF Object File Emission (.o file)
    │
    ▼
Linker (ld/cc → executable)
```

### Instruction Selection

Use a simple tree-pattern matching approach:
- Walk the IR in reverse postorder
- For each instruction, match patterns and emit machine instructions
- Start with 1:1 mapping (one IR instruction → one or more machine instructions)
- Add combined patterns later (e.g., fused multiply-add, address mode folding)

### Register Allocation

Start with **linear scan** register allocation:
- Compute liveness intervals for each virtual register
- Sort by start point
- Walk through intervals, assigning physical registers greedily
- When registers run out, spill the interval ending latest
- Handle fixed registers (for calling conventions) as pre-colored intervals

Linear scan is simpler than graph coloring and good enough for -O1 quality code. Graph coloring can be added later as an optimization.

### Calling Convention (System V AMD64 ABI / AAPCS64)

**x86-64 System V ABI:**
- Integer args: RDI, RSI, RDX, RCX, R8, R9 (then stack)
- Float args: XMM0-XMM7
- Return: RAX (integer), XMM0 (float)
- Callee-saved: RBX, RBP, R12-R15
- Stack aligned to 16 bytes at call site

**AArch64 AAPCS64:**
- Integer args: X0-X7 (then stack)
- Float args: V0-V7
- Return: X0 (integer), V0 (float)
- Callee-saved: X19-X28, V8-V15
- Stack aligned to 16 bytes, grows down

### ELF Object Files

Use the `object` crate to write ELF object files. This handles:
- Section creation (.text, .data, .rodata, .bss)
- Symbol table entries
- Relocation entries (for function calls, global references)
- The `object` crate abstracts over ELF32/64 and architecture differences

---

## Acceptance Criteria

- [ ] `forge build hello.c` produces a working executable that prints "Hello, World!"
- [ ] Integer arithmetic (add, sub, mul, div, mod) works correctly
- [ ] Function calls with multiple arguments follow the ABI
- [ ] Global variables are accessible
- [ ] Pointers and pointer arithmetic work
- [ ] Structs are laid out with correct alignment and field offsets
- [ ] If/else, while, for, switch produce correct control flow
- [ ] Local variables are allocated on the stack correctly
- [ ] Works on x86-64 (Ubuntu) and AArch64 (cross-compile or native on Mac)
- [ ] Can compile and run a simple program using printf from libc
- [ ] All generated code passes basic correctness tests (compute factorial, fibonacci, etc.)

---

## Claude Code Prompts

### Prompt 7.1 — Machine instruction types and shared infrastructure

```
Create forge_codegen, forge_x86_64, and forge_aarch64 crates in the workspace.

In forge_codegen/src/lib.rs, define shared codegen infrastructure:

1. MachineInst trait/struct — a generic representation of a machine instruction:
   - opcode: target-specific opcode
   - operands: Vec<MachineOperand>
   - implicit_defs: registers implicitly written
   - implicit_uses: registers implicitly read

2. MachineOperand enum:
   - Register(PhysReg or VirtReg)
   - Immediate(i64)
   - Memory { base: Reg, offset: i32, index: Option<Reg>, scale: u8 }
   - Label(String)
   - StackSlot(u32)

3. Register types:
   - VirtReg(u32) — before register allocation
   - PhysReg { class: RegClass, index: u8 } — after register allocation
   - RegClass enum: GPR, FPR (general purpose, floating point)

4. StackFrame:
   - local_slots: Vec<StackSlot> with sizes and alignments
   - spill_slots: allocated during regalloc
   - total_frame_size() -> u32 (computed after regalloc)
   - arg_area_size: for outgoing function arguments on stack

5. CallingConvention trait:
   - arg_registers() -> &[PhysReg]
   - return_register(ty) -> PhysReg
   - callee_saved() -> &[PhysReg]
   - classify_arg(index, ty) -> ArgLocation (Register or Stack offset)

Write placeholder implementations. No actual x86/AArch64 specifics yet.
```

### Prompt 7.2 — x86-64 instruction selection

```
Implement x86-64 instruction selection in forge_x86_64.

Define x86-64 specific types:
1. X86Opcode enum: Mov, Add, Sub, Imul, Idiv, And, Or, Xor, Shl, Shr, Sar, Cmp, Test, Jmp, Jcc, Call, Ret, Push, Pop, Lea, Movzx, Movsx, Cdq, SetCC, Neg, Not, Nop
2. X86Register: RAX, RBX, RCX, RDX, RSI, RDI, RBP, RSP, R8-R15 (plus 32-bit, 16-bit, 8-bit sub-registers)
3. X86CC (condition code): E, NE, L, LE, G, GE, B, BE, A, AE

Implement select_instruction(ir_inst: &Instruction) -> Vec<X86Inst>:

Mapping:
- iconst → mov reg, imm
- iadd → add reg, reg (or lea for three-operand add)
- isub → sub reg, reg
- imul → imul reg, reg
- idiv/urem → set up RDX:RAX, idiv/div, result in RAX (quotient) or RDX (remainder)
- band/bor/bxor → and/or/xor
- shl/sshr/ushr → shl/sar/shr (shift amount in CL register)
- icmp → cmp + setcc
- load → mov reg, [addr]
- store → mov [addr], reg
- stack_alloc → lea reg, [rbp - offset]
- gep → lea with scaled index
- call → push args to registers per ABI, call, read result from RAX
- branch → cmp + jcc
- jump → jmp
- return → mov result to RAX, ret
- sext → movsx, zext → movzx, trunc → mov with smaller register

Implement the System V x86-64 calling convention.

Write tests:
- Select instructions for a function returning a constant
- Select instructions for add, sub, mul
- Select instructions for a function call with 3 arguments
- Verify the selected instruction sequence is plausible (not verifying encoding yet)
```

### Prompt 7.3 — Register allocation (linear scan)

```
Implement linear scan register allocation in forge_codegen/src/regalloc.rs.

1. Liveness analysis:
   - For each virtual register, compute the live interval (first def to last use)
   - Handle block boundaries: if a vreg is live-out of a block and live-in to a successor, extend the interval

2. Linear scan algorithm:
   - Sort intervals by start point
   - Maintain an "active" set of intervals currently assigned to physical registers
   - For each interval:
     a. Expire old intervals (remove intervals that ended before current start, freeing their registers)
     b. Try to allocate a physical register from the available set
     c. If no register available, spill: pick the interval with the latest end point (current or active), assign it a stack slot, insert load/store at use/def points
   - Respect pre-colored intervals: some instructions require specific registers (e.g., div uses RDX:RAX, shifts use CL)

3. Spill code insertion:
   - At each spill point, insert a store to the spill slot after the def
   - At each reload point, insert a load from the spill slot before the use
   - Allocate spill slots in the stack frame

4. Register constraints:
   - x86-64 GPRs available for allocation: RAX, RCX, RDX, RSI, RDI, R8-R11 (caller-saved), RBX, R12-R15 (callee-saved, prefer to avoid)
   - Reserve RSP and RBP for stack management
   - Handle calling convention: before a call, allocate argument registers; after a call, the result is in RAX

Write tests:
- A function with fewer live values than registers → no spills
- A function needing spills → verify spill/reload instructions are inserted
- A function with a call → verify callee-saved registers are preserved
```

### Prompt 7.4 — Machine code encoding and ELF emission

```
Implement x86-64 machine code encoding and ELF object file emission.

Machine code encoding (forge_x86_64/src/encode.rs):
1. For each X86Inst, produce the raw bytes of the encoded instruction
2. Handle REX prefixes (for 64-bit operand size, R8-R15 register access)
3. Handle ModR/M and SIB bytes for memory operands
4. Handle immediate operands of different sizes (8, 16, 32, 64 bit)
5. Handle RIP-relative addressing for global symbols
6. For branch/call targets that aren't yet known, emit placeholder bytes and record relocations

Key encodings to implement:
- mov reg, reg (0x89/0x8B with REX.W)
- mov reg, imm (0xB8+ with REX.W for 64-bit)
- add/sub/and/or/xor reg, reg (0x01/0x29/0x21/0x09/0x31 families)
- imul reg, reg (0x0F 0xAF)
- idiv reg (0xF7 /7)
- shl/shr/sar reg, cl (0xD3)
- cmp reg, reg (0x39)
- jcc rel32 (0x0F 0x80+cc)
- jmp rel32 (0xE9)
- call rel32 (0xE8)
- ret (0xC3)
- push/pop (0x50+/0x58+)
- mov [base+disp], reg and mov reg, [base+disp]
- lea reg, [base+index*scale+disp]
- nop (0x90)

ELF emission (forge_x86_64/src/emit.rs):
Use the `object` crate (add as dependency):
1. Create a writable Object for x86-64 ELF
2. Create .text section, write encoded machine code
3. Create .data/.rodata sections for global variables and string literals
4. Add symbol table entries for each function (global symbols)
5. Add relocation entries for calls to external functions (like printf)
6. Write the object file to disk

Linker invocation:
- Run `cc -o output input.o` (using the system C compiler as linker to pull in libc)
- Or `ld -o output input.o -lc -dynamic-linker /lib64/ld-linux-x86-64.so.2`

End-to-end test: compile a C program that does `int main() { return 42; }`, run it, verify exit code is 42.
```

### Prompt 7.5 — AArch64 backend

```
Implement the AArch64 backend in forge_aarch64, following the same pattern as x86-64.

AArch64 instruction selection:
- iconst → mov/movz/movk (for large immediates, use movz + movk sequence)
- iadd → add, isub → sub
- imul → mul, idiv → sdiv, urem → udiv + msub
- band → and, bor → orr, bxor → eor
- shl → lsl, sshr → asr, ushr → lsr
- icmp → cmp + cset
- load → ldr, store → str
- call → bl, return → ret
- branch → b.cond, jump → b

AArch64 calling convention (AAPCS64):
- Args in X0-X7, float in V0-V7
- Return in X0 / V0
- Callee-saved: X19-X28, V8-V15
- Frame pointer: X29, link register: X30

Machine code encoding:
AArch64 has fixed 32-bit instruction encoding, which is simpler than x86:
- Data processing (register): [sf][opc][shift][Rm][imm6][Rn][Rd]
- Data processing (immediate): varies by instruction group
- Load/store: [size][opc][offset][Rn][Rt]
- Branch: [opc][imm26] or [opc][cond][imm19]

Implement encoding for the core instructions: add, sub, mul, sdiv, and, orr, eor, lsl, asr, lsr, cmp, b, b.cond, bl, ret, mov, ldr, str, stp, ldp.

ELF emission: same approach with `object` crate but for AArch64 ELF.

Testing: same tests as x86-64. If on an x86 machine, tests can cross-compile and check the object file structure (use `objdump` or similar) without running the binary. If AArch64 hardware is available, run the binary.
```

### Prompt 7.6 — End-to-end integration and libc interaction

```
Wire the code generator into the full pipeline and create end-to-end tests.

1. Update forge_driver:
   - Full pipeline: Lex → Preprocess → Parse → Sema → IR → Optimize → Codegen → Link
   - `forge build file.c` produces an executable
   - `forge build file.c -o output` specifies output name
   - Auto-detect target: default to host architecture, add --target=x86-64 and --target=aarch64 flags

2. Support calling libc functions:
   - When the C program calls printf, malloc, etc., emit an external symbol reference
   - The linker resolves these against libc
   - Handle string literal constants (put in .rodata, reference by address)

3. End-to-end test programs:
   - tests/run/hello.c: printf("Hello, World!\n"); — verify output
   - tests/run/arithmetic.c: compute and return 6*7, verify exit code 42
   - tests/run/factorial.c: compute factorial(10), verify result
   - tests/run/fibonacci.c: compute fib(20), verify result
   - tests/run/struct.c: create a struct, access members, return a field
   - tests/run/array.c: fill an array, sum it, return the sum
   - tests/run/pointer.c: pointer arithmetic tests
   - tests/run/global.c: global variables, modify and read back

4. The test harness should:
   - Compile each .c file with forge
   - Run the resulting executable
   - Check exit code and/or stdout against expected values

5. Cargo clippy, all tests pass. This is a MAJOR milestone — Forge is now a working compiler.
```

---

## Notes

- **Start with x86-64.** Since the primary dev machine is x86-64, get it working there first, then port to AArch64. The codegen infrastructure is shared; only instruction selection and encoding differ.
- **Don't optimize the codegen yet.** The e-graph handles high-level optimization. The codegen should produce correct, simple code. Better instruction selection (address mode folding, conditional move, etc.) can be added incrementally.
- **The `object` crate is a lifesaver.** Writing an ELF emitter from scratch is tedious and error-prone. Let the crate handle the binary format.
- **Linking against libc is the simplest path** to I/O. Once printf works, we can test everything.
