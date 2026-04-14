# Phase 9 — Incremental Compilation

**Depends on:** Phase 5 (IR), Phase 7 (Codegen)
**Unlocks:** Phase 11 (Conformance — faster iteration)
**Estimated duration:** 10–18 days

---

## Goal

Implement fine-grained incremental compilation so that when a source file changes, Forge recompiles only the affected functions rather than the entire translation unit. This is the developer experience differentiator — fast rebuilds for large codebases.

---

## Deliverables

1. **`forge_incr` crate** — incremental compilation engine
2. **Content-addressable cache** — hash-based caching of compilation artifacts at function granularity
3. **Dependency tracking** — track which functions depend on which types/globals/macros
4. **Persistent cache** — on-disk cache that survives across compiler invocations
5. **Cache invalidation** — correctly invalidate when headers, macros, or types change

---

## Technical Design

### Architecture

```
Source change detected
    │
    ▼
Re-lex & re-preprocess changed file
    │
    ▼
Compare new token stream to cached
    │
    ▼
Identify changed top-level declarations
    │
    ▼
For each changed function:
    ├─ Re-parse
    ├─ Re-analyze (sema)
    ├─ Re-lower to IR
    ├─ Re-optimize (e-graph)
    └─ Re-generate machine code
    │
    ▼
Link: combine new objects with cached objects
    │
    ▼
Output executable
```

### Cache Key Design

Each compilation artifact is stored with a content hash:

```rust
struct CacheKey {
    // Hash of: function source tokens + all types it references + compiler flags
    content_hash: [u8; 32],  // blake3 hash
}

struct CachedFunction {
    key: CacheKey,
    ir: Vec<u8>,           // serialized Forge IR
    machine_code: Vec<u8>, // encoded machine instructions
    relocations: Vec<Relocation>,
    dependencies: Vec<DependencyRef>,  // what this function depends on
}
```

### Dependency Graph

Track dependencies at the function level:
- **Type dependencies:** if function `foo()` uses `struct Bar`, it depends on the definition of `Bar`
- **Global dependencies:** if `foo()` reads global variable `g`, it depends on `g`'s type and initializer
- **Macro dependencies:** if the preprocessed tokens of `foo()` changed due to a macro redefinition, it needs recompilation
- **Header dependencies:** if a header included by the file changes, re-preprocess and check what changed

### Cache Storage

Use a directory-based cache:
```
.forge-cache/
├── meta.json           (cache metadata, version)
├── functions/
│   ├── a1b2c3d4.bin    (cached function artifact, named by hash)
│   └── ...
└── deps/
    ├── main.c.deps     (dependency info per source file)
    └── ...
```

---

## Acceptance Criteria

- [ ] First build populates the cache
- [ ] Second build with no changes is near-instant (just reads cache + links)
- [ ] Changing a function body recompiles only that function
- [ ] Changing a struct definition recompiles all functions using that struct
- [ ] Changing a header recompiles affected translation units
- [ ] Cache correctly invalidated when compiler flags change
- [ ] Cache can be cleared with `forge clean`
- [ ] Measurable speedup on a 10-file project when one file changes

---

## Claude Code Prompts

### Prompt 9.1 — Cache infrastructure and hashing

```
Create the forge_incr crate in the Forge workspace. Add `blake3` as a dependency for hashing.

Implement the core caching infrastructure:

1. ContentHash — a [u8; 32] wrapper with Display (hex) and Hash/Eq
2. CacheKey — holds a ContentHash identifying a specific compilation artifact
3. CacheStore — manages the on-disk cache directory:
   - new(cache_dir: PathBuf) -> CacheStore
   - get(key: &CacheKey) -> Option<Vec<u8>> — read cached artifact
   - put(key: &CacheKey, data: &[u8]) -> Result<()> — write cached artifact
   - has(key: &CacheKey) -> bool — check if artifact exists
   - clear() -> Result<()> — delete all cached data
   - stats() -> CacheStats { entries: usize, total_bytes: u64 }
   - Default cache directory: .forge-cache/ in the project root

4. Hashing functions:
   - hash_tokens(tokens: &[Token]) -> ContentHash — hash a token stream
   - hash_function_context(func_tokens: &[Token], type_deps: &[ContentHash], flags: &CompilerFlags) -> CacheKey
   - hash_file(path: &Path) -> ContentHash — hash file contents for change detection

5. Serialization for cached artifacts:
   - IR serialization: implement Serialize/Deserialize (use bincode) for forge_ir types
   - Machine code: already bytes, store directly with relocation metadata

Write tests:
- Same tokens produce same hash
- Different tokens produce different hash
- Cache store round-trip: put then get returns same data
- Cache clear empties the directory
```

### Prompt 9.2 — Dependency tracking

```
Implement fine-grained dependency tracking in forge_incr.

1. DependencyRef enum:
   - TypeDef { name: String, hash: ContentHash } — depends on a struct/union/enum/typedef definition
   - GlobalVar { name: String, hash: ContentHash } — depends on a global variable
   - Function { name: String, hash: ContentHash } — depends on another function (for inlining)
   - Header { path: PathBuf, hash: ContentHash } — depends on a header file

2. DependencyGraph:
   - Tracks which functions depend on which types/globals
   - Built during semantic analysis: when sema processes a function, record every type and symbol it touches
   - Stored per-file in the cache

3. Invalidation logic:
   - Given a set of changes (modified files, modified types), compute the set of functions that need recompilation
   - Transitive: if struct A contains struct B, and B changes, functions using A are also invalidated
   - conservative: if we can't determine the impact, recompile everything in the affected file

4. Integration points:
   - After preprocessing: compare file-level token hash to cached hash. If same, skip entirely.
   - After parsing: for each function, compute its token hash. If a function's tokens didn't change AND none of its dependencies changed, use the cached artifact.
   - After sema: update the dependency records for each function.

Write tests:
- Function with no type dependencies: changing another function in same file doesn't invalidate it
- Function using a struct: changing the struct invalidates the function
- Transitive dependency: A uses B, B uses C, changing C invalidates A and B
```

### Prompt 9.3 — Incremental pipeline integration

```
Integrate incremental compilation into the Forge driver.

1. Update forge_driver compile pipeline:
   - Before full compilation, check the cache
   - For each source file: hash it, check if cached
   - For unchanged files: load cached artifacts
   - For changed files: identify which functions changed, recompile only those
   - Merge cached and fresh artifacts
   - Link everything together

2. Add CLI flags:
   - --incremental (default: on): enable incremental compilation
   - --no-incremental: force full recompilation
   - forge clean: clear the cache

3. Create a multi-file test project:
   - main.c, util.c, util.h, math.c, math.h
   - First build: full compilation, cache populated
   - Touch main.c: only main.c recompiled
   - Touch util.h: main.c and util.c recompiled (both include it)
   - No changes: link only, near-instant

4. Add timing output:
   - --time flag shows per-phase timing
   - Show cache hit/miss statistics

5. Measure and log the speedup on the test project.
```

---

## Notes

- Incremental compilation at the function level is ambitious. Most compilers do file-level caching at best. Start with file-level, then refine to function-level once the infrastructure is solid.
- The preprocessor is the biggest challenge for incrementality: a macro defined in a header can affect any function. The dependency graph must account for preprocessor state.
- Consider using `salsa` (a Rust crate for incremental computation) if the manual dependency tracking becomes too complex.
- The cache must be invalidated when compiler flags change (optimization level, target arch, etc.).
