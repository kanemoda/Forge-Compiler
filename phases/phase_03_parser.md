# Phase 3 — Parser & AST

**Depends on:** Phase 2 (Preprocessor)
**Unlocks:** Phase 4 (Semantic Analysis)
**Estimated duration:** 10–18 days

---

## Goal

Build a hand-written recursive descent parser that consumes preprocessed tokens and produces a complete C17 AST. The parser must handle the full C17 grammar including all declaration forms, statement types, and expression syntax. Error recovery is critical — the parser should not bail on the first error.

---

## Deliverables

1. **`forge_parser` crate** — recursive descent parser producing an AST
2. **Complete C17 AST types** — covering all declarations, statements, and expressions
3. **Pratt parser** for expressions (correct precedence without deep recursion)
4. **Error recovery** — synchronize on semicolons, braces, and other landmarks
5. **Comprehensive tests** covering every C17 grammar production

---

## Technical Design

### AST Node Design

Every AST node carries a `Span` for diagnostics. Use an arena allocator (`bumpalo` or typed-arena) for AST nodes to avoid excessive heap allocations.

```rust
pub type NodeId = u32;  // index into arena

pub struct TranslationUnit {
    pub declarations: Vec<ExternalDeclaration>,
}

pub enum ExternalDeclaration {
    FunctionDef(FunctionDef),
    Declaration(Declaration),
}

pub struct FunctionDef {
    pub return_type: TypeSpecifier,
    pub name: String,
    pub params: Vec<ParamDecl>,
    pub body: CompoundStmt,
    pub span: Span,
}

pub struct Declaration {
    pub specifiers: DeclSpecifiers,
    pub declarators: Vec<InitDeclarator>,
    pub span: Span,
}
// ... etc
```

### Key Grammar Challenges

**The declaration/expression ambiguity:**
In C, `x * y;` could be a declaration (pointer to x named y) or an expression (multiply x by y). The parser resolves this by consulting a "typedef table" — if `x` has been declared as a `typedef`, it's a declaration. This means the parser needs to track typedef names as it parses. This is the famous C parsing problem.

**Solution:** Maintain a set of typedef names in the parser state. When parsing a statement at block scope, peek at the first token:
- If it's a type keyword or a known typedef name → parse as declaration
- Otherwise → parse as expression statement

**Declarator syntax:**
C declarators are infamously complex: `int (*(*fp)(int))[10]` is a pointer to a function taking int returning a pointer to an array of 10 ints. Parse declarators recursively: a declarator is an optional pointer prefix, then a direct-declarator (name, parenthesized declarator, array suffix, function suffix).

### Expression Parsing (Pratt)

Use a Pratt parser (top-down operator precedence) for expressions. This handles all C operators with correct precedence and associativity without needing one function per precedence level.

C17 precedence levels (high to low):
1. Postfix: `()`, `[]`, `.`, `->`, `++`, `--`, compound literals
2. Unary: prefix `++`/`--`, `&`, `*`, `+`, `-`, `~`, `!`, `sizeof`, `_Alignof`, cast
3. Multiplicative: `*`, `/`, `%`
4. Additive: `+`, `-`
5. Shift: `<<`, `>>`
6. Relational: `<`, `>`, `<=`, `>=`
7. Equality: `==`, `!=`
8. Bitwise AND: `&`
9. Bitwise XOR: `^`
10. Bitwise OR: `|`
11. Logical AND: `&&`
12. Logical OR: `||`
13. Conditional: `? :`
14. Assignment: `=`, `+=`, etc. (right-associative)
15. Comma: `,`

---

## Acceptance Criteria

- [ ] Parse simple function definitions: `int main() { return 0; }`
- [ ] Parse all statement types: if/else, while, do-while, for, switch/case, goto/label, return, break, continue, compound
- [ ] Parse all declaration forms: variables, functions, typedefs, structs, unions, enums
- [ ] Parse complex declarators: pointers, arrays, function pointers, nested combinations
- [ ] Parse all expression operators with correct precedence
- [ ] Parse struct/union definitions with bit-fields
- [ ] Parse enum definitions with explicit values
- [ ] Parse initializer lists with designators: `{ .x = 1, [0] = 2 }`
- [ ] Parse `_Generic` selections
- [ ] Parse `_Static_assert`
- [ ] Parse compound literals: `(int[]){1, 2, 3}`
- [ ] Typedef names are tracked and used to resolve the declaration/expression ambiguity
- [ ] Error recovery: a syntax error in one function doesn't prevent parsing of subsequent functions
- [ ] Can parse simplified versions of real C files (not yet with full semantic correctness)

---

## Claude Code Prompts

### Prompt 3.1 — AST types

```
Create the forge_parser crate in the Forge workspace. Start by defining the complete AST type hierarchy for C17. DO NOT write the parser yet — just the types.

Define the AST in forge_parser/src/ast.rs:

TranslationUnit — Vec<ExternalDeclaration>

ExternalDeclaration:
- FunctionDef { specifiers, declarator, body, span }
- Declaration(Declaration)

Declaration { specifiers: DeclSpecifiers, init_declarators: Vec<InitDeclarator>, span }

DeclSpecifiers — a struct collecting:
- storage_class: Option<StorageClass> (auto, register, static, extern, typedef, _Thread_local)
- type_specifier: TypeSpecifier
- type_qualifiers: Vec<TypeQualifier> (const, volatile, restrict, _Atomic)
- function_specifiers: Vec<FunctionSpecifier> (inline, _Noreturn)
- alignment: Option<AlignSpec>

TypeSpecifier — an enum:
- Void, Char, Short, Int, Long, Float, Double, Signed, Unsigned, Bool, Complex
- Struct(StructDef), Union(StructDef), Enum(EnumDef)
- TypedefName(String)
- Atomic(Box<TypeSpecifier>)
- Multiple combined specifiers should be normalized later in sema, but parser collects them as a list

InitDeclarator { declarator: Declarator, initializer: Option<Initializer> }

Declarator { pointer_depth: Vec<Vec<TypeQualifier>>, direct: DirectDeclarator }

DirectDeclarator — enum:
- Identifier(String, Span)
- Parenthesized(Box<Declarator>)
- Array { base: Box<DirectDeclarator>, size: Option<Box<Expr>>, qualifiers, is_static: bool }
- Function { base: Box<DirectDeclarator>, params: Vec<ParamDecl>, is_variadic: bool }

ParamDecl { specifiers: DeclSpecifiers, declarator: Option<Declarator> }

Stmt — enum:
- Compound(Vec<BlockItem>)
- Expr(Option<Expr>)  // expression statement or empty statement
- If { condition, then_branch, else_branch }
- While { condition, body }
- DoWhile { body, condition }
- For { init, condition, update, body }
- Switch { expr, body }
- Case { value, body }
- Default(Box<Stmt>)
- Return(Option<Expr>)
- Break, Continue
- Goto(String)
- Label { name, stmt }
- (all with Span)

BlockItem: Declaration(Declaration) | Statement(Stmt)

Expr — enum:
- IntLiteral(u64), FloatLiteral(f64), CharLiteral(u32), StringLiteral(String)
- Ident(String)
- BinaryOp { op, left, right }
- UnaryOp { op, operand }
- PostfixOp { op, operand }
- Conditional { condition, then_expr, else_expr }
- Assignment { op, target, value }
- FunctionCall { callee, args }
- MemberAccess { object, member, is_arrow: bool }
- ArraySubscript { array, index }
- Cast { type_name: TypeName, expr }
- SizeofExpr(Box<Expr>), SizeofType(TypeName)
- AlignofType(TypeName)
- CompoundLiteral { type_name: TypeName, initializer_list }
- GenericSelection { controlling_expr, associations }
- Comma(Vec<Expr>)
- (all with Span)

Initializer: Expr(Expr) | List(Vec<DesignatedInit>)
DesignatedInit { designators: Vec<Designator>, init: Initializer }
Designator: Index(Expr) | Field(String)

StructDef { name: Option<String>, members: Option<Vec<StructMember>> }
StructMember { specifiers, declarator, bit_width: Option<Expr> }
EnumDef { name: Option<String>, enumerators: Option<Vec<Enumerator>> }
Enumerator { name: String, value: Option<Expr> }

All nodes should derive Clone and Debug. Every node with a span should have a pub span: Span field.

TypeName { specifiers: DeclSpecifiers, abstract_declarator: Option<Declarator> }

Put the BinaryOp, UnaryOp, AssignOp enums in a separate ast_ops.rs file. Include all C17 operators.

Do not build the parser yet. Just the types. Write a few tests constructing AST nodes manually to verify the types compile correctly.
```

### Prompt 3.2 — Expression parser (Pratt)

```
Implement a Pratt expression parser in forge_parser/src/expr.rs.

Create a Parser struct in forge_parser/src/parser.rs that holds:
- tokens: Vec<Token> (preprocessed)
- pos: usize (current position)
- typedefs: HashSet<String> (known typedef names)
- diagnostics: Vec<Diagnostic>

Helper methods on Parser:
- peek() -> &Token
- advance() -> Token
- expect(kind: TokenKind) -> Result<Token, Diagnostic>
- at(kind: TokenKind) -> bool
- eat(kind: TokenKind) -> Option<Token>

Implement expression parsing using the Pratt technique:
- parse_expr() — entry point, parses a full expression including comma operator
- parse_assignment_expr() — assignment and above
- parse_pratt(min_binding_power: u8) — the core loop
- prefix_binding_power(op) and postfix_binding_power(op) and infix_binding_power(op) functions
- Return Expr AST nodes

Handle ALL C17 expression forms:
- Primary: identifiers, integer/float/char/string literals, parenthesized expressions
- Postfix: function call, array subscript, member access (. and ->), post-increment/decrement
- Unary: pre-increment/decrement, address-of (&), dereference (*), unary +/-, bitwise not (~), logical not (!)
- sizeof (both expression and type forms)
- _Alignof(type)
- Cast: (type-name) expr — tricky! Must disambiguate from parenthesized expression. Use the typedef table.
- All binary operators with correct precedence (see the 15 levels in the phase doc)
- Conditional: ternary ? : (right-associative)
- Assignment operators (right-associative)
- Comma operator (lowest precedence)
- Compound literals: (type-name) { initializer-list }
- _Generic(expr, type: expr, type: expr, default: expr)

The cast vs. parenthesized expression ambiguity: if the token after '(' is a type keyword or typedef name AND the token after the closing ')' looks like the start of an expression (not an operator), treat it as a cast. Otherwise, it's a parenthesized expression.

Write tests for:
- Simple arithmetic: 1 + 2 * 3 → correct precedence tree
- All unary operators
- Function calls with multiple arguments
- Nested member access: a.b->c[d]
- Ternary: a ? b : c ? d : e (right-associative)
- Assignment chain: a = b = c (right-associative)
- Cast expression: (int)x
- sizeof expression and sizeof type
- Compound literal
```

### Prompt 3.3 — Declaration parser

```
Implement declaration parsing in forge_parser.

This is the most complex part of the C parser. A C declaration has the form:
  declaration-specifiers init-declarator-list? ;

Implement:

1. parse_declaration_specifiers() — collect storage class, type specifiers, type qualifiers, function specifiers, alignment specifiers. Multiple type specifiers combine: `unsigned long long int` is valid. Track what specifiers have been seen to detect conflicts (e.g., `float double` is an error).

2. parse_declarator() — the recursive declarator syntax:
   - Optional pointer prefix: * with optional type qualifiers, can be chained
   - Direct declarator: identifier, or (declarator) for grouping
   - Suffixes: [size] for arrays (with optional static, qualifiers, * for VLA), (params) for functions

3. parse_parameter_list() — comma-separated parameter declarations, optional ... for variadic

4. parse_init_declarator_list() — declarator optionally followed by = initializer, comma-separated

5. parse_initializer() — either a single assignment expression, or { initializer-list } with optional designated initializers (.field = value, [index] = value)

6. Struct/union definitions: parse_struct_or_union_specifier() — handles both `struct foo { ... }` and `struct foo` (forward reference). Members can have bit-fields: `int x : 3;`

7. Enum definitions: parse_enum_specifier() — handles `enum foo { A, B = 5, C }`

8. Typedef tracking: when a declaration has typedef storage class, add the declared names to the typedef set. This is CRITICAL for resolving the declaration/expression ambiguity.

9. parse_type_name() — for use in sizeof(type), casts, compound literals. Like a declaration but with no name (abstract declarator).

Write tests:
- Simple: int x;
- Multiple declarators: int x, y, *z;
- Function declaration: int foo(int a, char *b);
- Complex declarator: int (*(*fp)(int))[10];
- Struct definition with members
- Enum with values
- Typedef and then use in later declaration
- Initializer list with designators
- Bit-fields
- Variable-length array: int arr[n];
```

### Prompt 3.4 — Statement and function parser

```
Implement statement and top-level parsing in forge_parser.

Statements — parse_statement():
1. Compound statement: { block-item-list }
   - block-item is either a declaration or a statement
   - New scope for typedefs (typedef names declared inside a block are only visible in that block)

2. If statement: if (expr) stmt [else stmt]
   - Handle dangling else correctly (else binds to nearest if)

3. While: while (expr) stmt

4. Do-while: do stmt while (expr);

5. For: for (init; condition; update) stmt
   - init can be a declaration (C99+) or expression

6. Switch: switch (expr) stmt
   - Case labels: case constant-expr :
   - Default: default :

7. Goto: goto identifier;
8. Label: identifier : stmt
9. Return: return [expr];
10. Break, Continue: break; continue;
11. Expression statement: expr; or empty statement: ;
12. _Static_assert(expr, string-literal);

Top-level — parse_translation_unit():
1. Loop parsing external declarations until EOF
2. External declaration is either a function definition or a declaration
3. Function definition: declaration-specifiers declarator compound-statement
   - Distinguished from a declaration by: if after the declarator we see '{' instead of ',' or ';' or '='

Error recovery:
- On unexpected token, emit a diagnostic
- Skip tokens until we reach a synchronization point: ';', '}', or the start of a new declaration (type keyword at start of line)
- The parser should recover and continue parsing after errors

Write tests:
- Function definition: int main() { return 0; }
- All statement types
- Nested blocks with local declarations
- For loop with declaration init: for (int i = 0; i < n; i++)
- Switch with case/default
- Multiple functions in one translation unit
- Error recovery: missing semicolon, parser continues
```

### Prompt 3.5 — Integration and full parse tests

```
Integrate the parser into the Forge driver and create comprehensive tests.

1. Update forge_driver to:
   - After preprocessing, parse the token stream into an AST
   - For `forge check`, print a pretty-printed AST dump (implement Display or a custom printer for AST nodes)
   - Add `forge parse <file.c>` subcommand for just parsing (no semantic analysis)

2. Implement a basic AST pretty-printer that can dump the AST in a readable tree format, e.g.:
   TranslationUnit
     FunctionDef "main" -> int
       CompoundStmt
         ReturnStmt
           IntLiteral 0

3. Create lit tests in tests/lit/parser/:
   - tests/lit/parser/declarations.c — all declaration forms
   - tests/lit/parser/expressions.c — expression precedence tests
   - tests/lit/parser/statements.c — all statement types
   - tests/lit/parser/structs.c — struct/union/enum definitions
   - tests/lit/parser/complex.c — a file using all features together
   - tests/lit/parser/errors.c — syntax errors with recovery

4. Parse test: create a simple but complete C program (30-50 lines using most language features) and verify it parses without errors.

5. Run cargo clippy, fix warnings. Ensure all tests pass.
```

---

## Notes

- The typedef tracking is the single most important correctness issue. If this breaks, the parser will misparse declarations as expressions and vice versa. Test this heavily.
- Don't try to do any type checking here — that's Phase 4. The parser just builds the tree.
- K&R-style function definitions (`int main(argc, argv) int argc; char **argv; { ... }`) are optional — they were deprecated long ago. Skip them unless there's time.
- GNU extensions (statement expressions, typeof, etc.) are out of scope for now.
