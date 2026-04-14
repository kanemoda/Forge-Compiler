# Phase 10 — Energy-Aware Code Generation

**Depends on:** Phase 7 (Codegen)
**Unlocks:** Phase 11 (Conformance — complete feature set)
**Estimated duration:** 20–35 days

---

## Goal

Implement an energy-aware code generation strategy that selects instructions and schedules code to minimize energy consumption, not just execution time or code size. This is the unique market angle — no mainstream compiler offers this as a first-class optimization target.

**This is the most research-heavy phase.** It requires energy models per microarchitecture, which may need to be built from measurements or literature.

---

## Deliverables

1. **`forge_energy` crate** — energy cost models and energy-aware scheduling
2. **Per-instruction energy costs** — energy tables for x86-64 and AArch64 microarchitectures
3. **Energy-aware instruction selection** — prefer lower-energy instruction sequences
4. **Energy-aware scheduling** — order instructions to minimize energy (reduce pipeline stalls, prefer efficient execution units)
5. **Energy-aware e-graph cost function** — alternative cost function that optimizes for energy instead of speed
6. **`-Oenergy` flag** — enable energy-optimized compilation

---

## Technical Design

### Energy Cost Model

Energy consumption per instruction depends on:
- **Instruction type:** ALU operations are cheap; memory accesses are expensive; division is very expensive
- **Data movement:** Register-to-register is cheapest; cache hit is moderate; cache miss is very expensive
- **Pipeline effects:** Branch mispredictions waste energy; dependent instruction chains stall and waste energy in speculative execution
- **Execution unit:** On ARM big.LITTLE, scheduling on efficiency cores uses less energy

### Approach 1: Static Energy Tables

Build tables mapping each instruction to an approximate energy cost (in picojoules or relative units):

```rust
struct EnergyCost {
    dynamic_energy: f64,    // energy consumed by the operation itself
    leakage_weight: f64,    // relative contribution to leakage (longer latency = more leakage)
}

fn x86_energy_cost(inst: &X86Inst) -> EnergyCost {
    match inst.opcode {
        Nop =>     EnergyCost { dynamic: 0.1, leakage: 0.0 },
        Mov =>     EnergyCost { dynamic: 0.3, leakage: 0.0 },
        Add =>     EnergyCost { dynamic: 0.5, leakage: 0.0 },
        Imul =>    EnergyCost { dynamic: 2.0, leakage: 1.0 },
        Idiv =>    EnergyCost { dynamic: 15.0, leakage: 10.0 },
        Load =>    EnergyCost { dynamic: 3.0, leakage: 2.0 },  // L1 hit
        // ...
    }
}
```

Source data from:
- Published microarchitecture studies (Agner Fog's instruction tables for latency/throughput, research papers for energy)
- Intel/AMD/ARM software optimization guides
- Research papers: "Energy-Efficient Computing" literature, ISCA/MICRO/HPCA proceedings

### Approach 2: E-Graph Energy Cost Function

Provide an alternative cost function for the e-graph extractor that minimizes total energy:

```rust
struct EnergyCostFunction { target: TargetArch }

impl CostFunction<ForgeIR> for EnergyCostFunction {
    fn cost(&mut self, enode: &ForgeIR, costs: impl FnMut(Id) -> f64) -> f64 {
        let energy = match enode {
            Iconst(_) => 0.0,
            Iadd(_) => 0.5,
            Imul(_) => 2.0,   // 4x more energy than add
            Idiv(_) => 15.0,  // very energy expensive
            Shl(_) => 0.3,    // shift is cheaper than add on most uarchs
            Load(_) => 3.0,
            // ...
        };
        enode.fold(energy, |sum, id| sum + costs(id))
    }
}
```

### Approach 3: Instruction Scheduling for Energy

Reorder instructions within a basic block to minimize energy:
- **Reduce pipeline stalls:** Stalls waste energy (leakage continues, no useful work). Schedule independent instructions between dependent chains.
- **Execution port balancing:** Distribute operations across execution ports to avoid port pressure (reduces dynamic energy from contention).
- **Memory access clustering:** Group memory accesses to improve cache behavior (fewer cache misses = less energy from DRAM access).
- **Register pressure:** High register pressure leads to spills, which are energy-expensive memory operations.

---

## Acceptance Criteria

- [ ] Energy cost tables exist for x86-64 (Zen 2 as reference) and AArch64 (Cortex-A76 or Apple M-series)
- [ ] `-Oenergy` flag enables energy-aware optimization
- [ ] E-graph extraction uses energy cost function when `-Oenergy` is set
- [ ] Instruction scheduling considers energy costs
- [ ] For energy-vs-speed tradeoffs, energy mode prefers the lower-energy option (e.g., shift over multiply even when multiply has same latency)
- [ ] A benchmark shows measurably different code generation between `-O2` and `-Oenergy`
- [ ] Documentation explains the energy model and its sources

---

## Claude Code Prompts

### Prompt 10.1 — Energy cost model infrastructure

```
Create the forge_energy crate in the Forge workspace.

Define the energy model infrastructure:

1. EnergyCost struct: { dynamic_pj: f64, leakage_pj: f64, total() -> f64 }
2. EnergyModel trait:
   - instruction_cost(inst) -> EnergyCost
   - memory_access_cost(access_type: L1Hit | L2Hit | L3Hit | DramAccess) -> EnergyCost
   - branch_mispredict_cost() -> EnergyCost
   - pipeline_stall_cost(cycles: u32) -> EnergyCost

3. X86Zen2EnergyModel implementing EnergyModel — energy costs for AMD Zen 2 (Ryzen 3600):
   - ALU ops: ~0.5 pJ
   - Multiply: ~2 pJ
   - Divide: ~15 pJ
   - L1 load: ~3 pJ
   - L2 load: ~10 pJ
   - L3 load: ~30 pJ
   - DRAM access: ~200 pJ
   (These are approximate relative values from research literature — document sources)

4. AArch64CortexA76EnergyModel implementing EnergyModel — similar table for ARM

5. A function to estimate total energy for a basic block: sum instruction costs, add estimated memory costs, add pipeline stall estimates.

Write tests verifying cost calculations. This is mainly data entry — getting the numbers right from literature.
```

### Prompt 10.2 — Energy-aware e-graph cost function

```
Implement an energy-aware cost function for the e-graph optimizer.

1. Create EnergyCostFunction in forge_energy that implements egg::CostFunction<ForgeIR>
2. It should use the EnergyModel to assign energy costs to each e-node
3. Extraction should minimize total energy rather than operation count or latency

4. Integrate with forge_egraph:
   - The optimize_function() function now takes an OptimizationGoal enum: Speed | Size | Energy
   - When Energy is selected, use EnergyCostFunction instead of the default ForgeCost
   - The rule set stays the same — only the extraction preference changes

5. Add the -Oenergy CLI flag to forge_cli that sets the optimization goal to Energy.

Write tests:
- With energy cost: x * 2 → x << 1 (shift is lower energy than multiply)
- With energy cost: x * 3 stays as x * 3 (no shift equivalent, and add+shift might cost more energy than multiply)
- Compare extracted code between Speed and Energy modes on a test function
```

### Prompt 10.3 — Energy-aware instruction scheduling

```
Implement a basic energy-aware instruction scheduler in forge_energy.

1. Within a basic block, reorder instructions to minimize estimated energy:
   - Build a dependency graph (data dependencies, memory ordering)
   - Use a list scheduler: at each step, choose the ready instruction that minimizes energy
   - Prefer scheduling independent instructions between dependent chains (reduces stall energy)
   - Prefer grouping memory accesses (better cache behavior)

2. Create a schedule_block_for_energy(block, energy_model) function that:
   - Takes a basic block of machine instructions
   - Returns a reordered basic block
   - Respects all data dependencies
   - Minimizes estimated total energy (instruction cost + estimated stall cost)

3. Integrate into the codegen pipeline:
   - After register allocation, before final encoding
   - Only active when -Oenergy is set

Write tests:
- A block with independent instructions: verify they get interleaved with dependent chains
- A block with memory accesses: verify clustering behavior
- Correctness: verify the reordered block produces the same results (test via execution)
```

### Prompt 10.4 — Benchmarking and documentation

```
Create benchmarks comparing energy-optimized vs speed-optimized code.

1. Create benchmark C programs in tests/benchmark/:
   - Matrix multiplication (100x100)
   - String processing (strlen, memcpy-like)
   - Sorting (quicksort on 10K integers)
   - Linked list traversal (pointer chasing)

2. Compile each with -O2 (speed) and -Oenergy (energy):
   - Dump the generated assembly for comparison
   - Count instruction types (multiplies vs shifts, etc.)
   - Report estimated total energy using the model

3. Write documentation in docs/energy_model.md:
   - Explain the approach and methodology
   - List the energy cost tables with sources
   - Discuss limitations (model is approximate, real energy depends on many factors)
   - Explain when -Oenergy is beneficial (battery-constrained devices, thermal-limited servers)

4. Create a summary that shows concrete examples where -Oenergy produces different (lower energy) code than -O2.
```

---

## Notes

- **This is research-grade work.** The energy numbers are approximate. Real energy measurement requires hardware power meters (Intel RAPL, ARM Energy Probe). The value is in the framework and the demonstration that a compiler *can* optimize for energy.
- **Start with the e-graph cost function.** It's the easiest integration point and provides real benefit with minimal complexity. Instruction scheduling for energy is the more sophisticated feature.
- **Battery life angle:** frame this for mobile and embedded developers. "Forge -Oenergy produces code that uses 15% less energy on Cortex-A76 compared to -O2." Even if the real number is 5%, it's a differentiator.
- **Future work:** DVFS (Dynamic Voltage and Frequency Scaling) hints, heterogeneous core scheduling (big.LITTLE aware), thermal throttling avoidance.
