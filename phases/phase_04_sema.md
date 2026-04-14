# Phase 4 — Semantic Analysis & Type System

**Depends on:** Phase 3 (Parser)
**Unlocks:** Phase 5 (IR)
**Estimated duration:** 10–18 days

---

## Goal

Implement full C17 type checking and semantic analysis. After this phase, every well-formed C17 program is accepted with a fully typed and validated AST, and every ill-formed program gets a clear, specific error message. This phase transforms the raw AST into a "typed AST" (TAST) where every expression has a known type.

---

## Deliverables

1. **`forge_sema` crate** — semantic analysis pass over the AST
2. **Type system** — full C17 type representation including qualifiers, pointers, arrays, functions, structs, enums
3. **Symbol table** — scoped name resolution with proper shadowing
4. **Implicit conversions** — integer promotions, usual arithmetic conversions, lvalue-to-rvalue, array-to-pointer decay, function-to-pointer decay
5. **Constant expression evaluation** — for array sizes, case labels, enum values, static assertions
6. **Typed AST** — every expression node annotated with its resolved type

---

## Technical Design

### Type Representation

```rust
pub enum Type {
    Void,
    Bool,
    Char { is_signed: Option<bool> },  // plain char signedness is impl-defined
    Short { is_unsigned: bool },
    Int { is_unsigned: bool },
    Long { is_unsigned: bool },
    LongLong { is_unsigned: bool },
    Float,
    Double,
    LongDouble,
    Pointer { pointee: Box<QualType> },
    Array { element: Box<QualType>, size: ArraySize },
    Function { return_type: Box<QualType>, params: Vec<QualType>, is_variadic: bool },
    Struct { name: Option<String>, def: Option<StructDefId> },
    Union { name: Option<String>, def: Option<UnionDefId> },
    Enum { name: Option<String>, def: Option<EnumDefId> },
    // Complex types if we support _Complex
}

pub struct QualType {
    pub ty: Type,
    pub is_const: bool,
    pub is_volatile: bool,
    pub is_restrict: bool,  // only valid on pointers
    pub is_atomic: bool,
}

pub enum ArraySize {
    Fixed(u64),
    Variable(ExprId),   // VLA
    Incomplete,          // extern int arr[];
    Star,                // int arr[*] in function prototype
}
```

### Symbol Table

```rust
pub struct SymbolTable {
    scopes: Vec<Scope>,  // stack of scopes
}

pub struct Scope {
    symbols: HashMap<String, Symbol>,
    kind: ScopeKind,  // File, Function, Block, Prototype
}

pub struct Symbol {
    pub name: String,
    pub ty: QualType,
    pub kind: SymbolKind,  // Variable, Function, TypeDef, EnumConstant, Parameter
    pub storage: StorageClass,
    pub linkage: Linkage,  // None, Internal, External
    pub span: Span,
}
```

### Key Semantic Rules

1. **Type specifier normalization:** `unsigned long long int` → `Type::LongLong { is_unsigned: true }`
2. **Integer promotions:** `char`, `short`, bit-fields narrower than `int` promote to `int`
3. **Usual arithmetic conversions:** when binary op mixes types, convert both to a common type
4. **Lvalue analysis:** assignment targets must be modifiable lvalues
5. **Array-to-pointer decay:** arrays in most expression contexts decay to pointers
6. **Function-to-pointer decay:** function names in expressions decay to function pointers
7. **Linkage rules:** `static` = internal linkage, `extern` = external linkage, default = depends on scope
8. **Compatible types:** struct/union types are compatible only with themselves, function types must match params
9. **Incomplete types:** forward-declared structs are incomplete until defined; can only form pointers to them

---

## Acceptance Criteria

- [ ] Type checking for all expression operators (correct operand types, result types)
- [ ] Implicit conversions inserted where needed
- [ ] Struct/union member access type-checked (member must exist)
- [ ] Function calls: argument count/types checked against parameter types
- [ ] Variable declarations: initializer type compatible with declared type
- [ ] Pointer arithmetic: pointer ± integer, pointer - pointer
- [ ] Array subscript: one operand is pointer, other is integer
- [ ] Assignment: target is modifiable lvalue, types compatible
- [ ] Return type matches function declaration
- [ ] Switch: controlling expression is integer type, case values are integer constants
- [ ] Goto: label must exist in the same function
- [ ] Break/continue: only inside loop or switch
- [ ] Typedef correctly creates type aliases
- [ ] Sizeof evaluated for all types (including structs with padding)
- [ ] Static assertions evaluated at compile time
- [ ] Duplicate symbol errors, type redefinition errors
- [ ] const correctness: assignment to const variable is an error
- [ ] Can analyze real C programs (30+ lines) with multiple functions, structs, enums

---

## Claude Code Prompts

### Prompt 4.1 — Type system and symbol table

```
Create the forge_sema crate in the Forge workspace. Start with the type system and symbol table.

Implement in forge_sema/src/types.rs:
1. Type enum covering all C17 types: Void, Bool, Char, Short, Int, Long, LongLong, Float, Double, LongDouble, Pointer, Array, Function, Struct, Union, Enum — per the design in the phase doc
2. QualType wrapping Type with const/volatile/restrict/atomic qualifiers
3. Methods on Type: is_integer(), is_floating(), is_arithmetic(), is_scalar(), is_pointer(), is_void(), is_complete(), is_function(), is_array(), size_of(target: &TargetInfo) -> Option<u64>, align_of(target: &TargetInfo) -> Option<u64>
4. TargetInfo struct with pointer size, int sizes, alignment info for x86-64 and AArch64 (LP64 model for both)
5. integer_rank() for integer promotion rules
6. A function usual_arithmetic_conversion(lhs: &Type, rhs: &Type) -> Type implementing the C standard conversion rules

Implement in forge_sema/src/scope.rs:
1. Symbol struct with name, QualType, SymbolKind (Variable, Function, Typedef, EnumConstant, Parameter, Label), StorageClass, Linkage, Span
2. Scope struct with a HashMap of symbols and a ScopeKind (File, Function, Block, Prototype)
3. SymbolTable with a Vec<Scope> as a scope stack
4. Methods: push_scope(), pop_scope(), declare(name, symbol) -> Result (error on duplicate in same scope), lookup(name) -> Option<&Symbol>, lookup_tag(name) for struct/union/enum tags (separate namespace)

Write tests for:
- Type size calculations for x86-64
- Usual arithmetic conversions (e.g., int + unsigned long → unsigned long)
- Symbol table scoping: declare in inner scope shadows outer, popping restores visibility
```

### Prompt 4.2 — Type specifier resolution and declaration analysis

```
Implement declaration analysis in forge_sema.

Create forge_sema/src/decl.rs:

1. resolve_type_specifiers(specifiers: &[TypeSpecifier]) -> Result<Type>
   - Normalize combined specifiers: "unsigned long long int" → LongLong { is_unsigned: true }
   - Error on incompatible combinations: "float double", "unsigned float", etc.
   - Handle struct, union, enum specifiers (look up or define in tag namespace)
   - Handle typedef names (look up in symbol table)

2. resolve_declarator(base_type: QualType, declarator: &Declarator) -> (String, QualType)
   - Apply pointer, array, and function suffixes to build the final type
   - Return the declared name and its complete type
   - Handle the recursive structure: `int (*fp)(int)` → pointer to function(int) → int

3. analyze_declaration(decl: &Declaration, scope: &mut SymbolTable) -> Result<TypedDeclaration>
   - Resolve specifiers, then each declarator
   - If typedef storage class: add to symbol table as Typedef
   - Otherwise: add as Variable/Function
   - Type-check initializers if present (prompt 4.3)
   - Handle storage class rules: static at file scope = internal linkage, extern = external linkage

4. analyze_struct_def(def: &StructDef) -> Result<StructLayout>
   - Check for duplicate member names
   - Compute offsets with alignment padding
   - Handle bit-fields
   - Handle flexible array member (last member can be incomplete array)

Write tests:
- Simple declarations: int x; → Variable x of type int
- Complex: int (*fp)(int, int); → Variable fp of type pointer to function
- Typedef: typedef int *IntPtr; IntPtr p; → p is pointer to int
- Struct with computed layout and alignment
```

### Prompt 4.3 — Expression type checking

```
Implement expression type checking in forge_sema.

Create forge_sema/src/expr.rs. For each expression kind, determine and annotate the result type:

1. Literals: int literals → int/long/etc. based on value and suffix. Float → double (or float/long double with suffix). Char → int. String → pointer to const char (or wide variants).

2. Identifiers: look up in symbol table, get the type. Error if undeclared. Apply array-to-pointer and function-to-pointer decay.

3. Binary operators:
   - Arithmetic (+, -, *, /, %): apply usual arithmetic conversions, result is the common type. % requires integer operands.
   - Shift (<<, >>): integer promotions on each operand independently, result type is promoted left operand type.
   - Relational (<, >, <=, >=): usual arithmetic conversions, result is int. Also valid for pointers of compatible types.
   - Equality (==, !=): like relational, also valid for pointer-to-void comparisons and null pointer comparisons.
   - Bitwise (&, |, ^): usual arithmetic conversions, requires integer operands.
   - Logical (&&, ||): operands must be scalar, result is int.
   - Pointer arithmetic: pointer + integer, integer + pointer, pointer - integer, pointer - pointer.

4. Unary operators:
   - &: operand must be lvalue, result is pointer to operand type
   - *: operand must be pointer, result is pointee type (lvalue)
   - +, -: integer promotion, result is promoted type
   - ~: integer promotion, integer operand required
   - !: scalar operand, result is int
   - ++, -- (pre and post): operand must be modifiable lvalue of arithmetic or pointer type

5. Assignment: target must be modifiable lvalue. Types must be compatible (with implicit conversion). Compound assignments (+=, etc.): equivalent to simple assignment of the binary operation.

6. Function calls: callee must be function type (or pointer-to-function). Check argument count. Apply default argument promotions for variadic args.

7. Member access (., ->): left operand must be struct/union (or pointer to for ->). Member must exist. Result type is member type.

8. Array subscript: one operand pointer, one integer. Result is dereferenced element type.

9. Cast: validate that the cast is legal (e.g., can't cast struct to int).

10. Sizeof: result type is size_t (unsigned long on LP64). If operand is expression, don't evaluate it.

11. Conditional (?:): conditions must be scalar. Second and third operands: if both arithmetic, usual conversions. If both pointers, must be compatible.

Write thorough tests for each expression type. Include tests for implicit conversions being correctly inserted (e.g., char + char → int + int → int).
```

### Prompt 4.4 — Statement analysis and function bodies

```
Implement statement analysis and complete function-body checking in forge_sema.

1. analyze_function_def(func: &FunctionDef, table: &mut SymbolTable) -> Result<TypedFunction>
   - Open a new function scope
   - Add parameters to scope
   - Analyze the body (compound statement)
   - Verify all code paths return a value (for non-void functions) — this can be a warning

2. analyze_stmt(stmt: &Stmt, table: &mut SymbolTable, context: &FnContext) -> Result<TypedStmt>
   - FnContext tracks: return type, whether we're in a loop, whether we're in a switch

   Compound: push scope, analyze each block item, pop scope
   If: condition must be scalar type. Analyze both branches.
   While/DoWhile: condition must be scalar. Body is in loop context.
   For: init can be declaration (new scope) or expression. Condition must be scalar.
   Switch: expression must be integer type. Body analyzed in switch context.
   Case: value must be integer constant expression, must be in switch context. Check for duplicate case values.
   Default: must be in switch context. Check for duplicate default.
   Return: if function returns void, return value is error. If non-void, expression must be compatible with return type.
   Break/Continue: must be in loop (continue) or loop/switch (break)
   Goto: record label reference, verify all labels exist at end of function
   Label: record label definition, check for duplicates

3. Constant expression evaluator — for case labels, array sizes, enum values, static assertions:
   - Evaluate integer constant expressions at compile time
   - Support: integer literals, enum constants, sizeof, arithmetic operators, casts between integer types
   - This is a subset of expression evaluation — no function calls, no variables

4. analyze_translation_unit(tu: &TranslationUnit) -> Result<TypedTranslationUnit>
   - File scope symbol table
   - Process each external declaration
   - At end: check that all used extern symbols are defined somewhere (or leave for linker)

Write tests:
- Function with correct return type checking
- Break/continue outside loop → error
- Duplicate case values → error
- Goto to undefined label → error
- Constant expression evaluation for enum values
- Full function analysis with locals, control flow, and returns
```

### Prompt 4.5 — Integration and real-program analysis

```
Integrate semantic analysis into the Forge driver and test with real C programs.

1. Update forge_driver to run sema after parsing:
   - Parse → Sema → (future: IR lowering)
   - forge check now reports type errors with beautiful diagnostics
   - Show the type of each expression in verbose mode

2. Create comprehensive test programs in tests/lit/sema/:
   - tests/lit/sema/types.c — basic type checking
   - tests/lit/sema/conversions.c — implicit conversion tests
   - tests/lit/sema/structs.c — struct layout, member access
   - tests/lit/sema/scopes.c — scoping and shadowing
   - tests/lit/sema/errors.c — expected error diagnostics

3. Write a 50-100 line C program that uses structs, enums, function pointers, loops, switch, arrays, and pointer arithmetic. It should type-check successfully.

4. Write a matching file of deliberately ill-formed code (each line is a type error). Verify each produces the right diagnostic.

5. Run cargo clippy, fix warnings. All tests pass.
```

---

## Notes

- This is where Forge starts feeling like a real compiler. Good diagnostics here are worth gold — "incompatible types in assignment: expected 'int *', found 'char'" is infinitely better than "type error".
- Don't implement every single obscure C rule. Focus on what matters for real programs: the integer promotion rules, pointer conversions, struct layout, and function call checking. VLA support can be minimal.
- The constant expression evaluator is shared with the preprocessor's #if evaluator but operates on AST nodes rather than tokens.
