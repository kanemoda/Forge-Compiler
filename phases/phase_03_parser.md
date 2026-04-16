# Phase 3 — Parser & AST (Revised)

**Depends on:** Phase 2 (Preprocessor) ✅ COMPLETE
**Unlocks:** Phase 4 (Semantic Analysis)
**Estimated duration:** 14–22 days

---

## Goal

Build a hand-written recursive descent parser that consumes preprocessed tokens and produces a complete C17 AST. The parser must handle the full C17 grammar including all declaration forms, statement types, and expression syntax. It must also tolerate GNU extensions present in system header output — after `#include <stdio.h>`, the preprocessed token stream contains `__attribute__`, `__extension__`, `__restrict`, etc. that the parser must not choke on.

Error recovery is critical — the parser should not bail on the first error.

---

## Key Design Decisions

### 1. Type specifier collection: list-based, not single-enum
C allows multiple type specifiers that combine: `unsigned long long int` is four separate specifier tokens. The parser collects these as a `Vec<TypeSpecifierToken>` and either resolves them during parsing or defers to sema. A single `TypeSpecifier` enum cannot represent mid-parse state.

### 2. No arena allocator yet — start with Box
Arena allocators (`bumpalo`, `typed-arena`) optimize allocation but add API complexity. Start with `Box<>` for recursive AST nodes. Profile in Phase 11; migrate to arena if allocation is a bottleneck. This avoids lifetime headaches while the AST design is still stabilizing.

### 3. GNU extension tolerance
System headers preprocessed by Forge produce tokens like `__attribute__((...))`, `__extension__`, `__asm__(...)`, `__restrict`, `__inline`, `__typeof__`, `__builtin_va_list`. The parser must handle these — at minimum by skipping `__attribute__((...))` balanced-paren groups and treating the rest as regular identifiers or qualifier keywords. This is NOT optional — without it, parsing any file that includes a system header fails.

### 4. Typedef tracking is the #1 correctness issue
`T * x;` is a pointer declaration if T is a typedef, or a multiplication expression if T is a variable. The parser must maintain a live set of typedef names, updated as declarations are parsed. Scoping matters — typedef names in inner blocks shadow outer ones and disappear when the block ends.

---

## Deliverables

1. **`forge_parser` crate** — recursive descent parser producing an AST
2. **Complete C17 AST types** — all declarations, statements, expressions
3. **Pratt parser** for expressions with correct C17 precedence
4. **Declaration parser** — including the full recursive declarator syntax
5. **GNU extension tolerance** — skip/handle common GCC/Clang extensions in system headers
6. **Error recovery** — synchronize on `;`, `}`, and declaration starts
7. **Comprehensive tests** — unit tests, lit tests, real-C parse tests

---

## Technical Design

### AST Overview

```
TranslationUnit
├── ExternalDeclaration
│   ├── FunctionDef { specifiers, declarator, body }
│   └── Declaration { specifiers, init_declarators }
│
├── DeclSpecifiers
│   ├── storage_class: Option<StorageClass>
│   ├── type_specifiers: Vec<TypeSpecifierToken>  ← LIST, not single
│   ├── type_qualifiers: Vec<TypeQualifier>
│   ├── function_specifiers: Vec<FunctionSpecifier>
│   └── alignment: Option<AlignSpec>
│
├── Declarator
│   ├── pointer: Vec<PointerQualifiers>
│   └── direct: DirectDeclarator (Ident | Paren | Array | Function)
│
├── Stmt (Compound | If | While | For | Switch | Return | Goto | Label | Expr | ...)
│
└── Expr (all via Pratt parser)
    ├── Literal, Ident, BinaryOp, UnaryOp, PostfixOp
    ├── FunctionCall, ArraySubscript, MemberAccess
    ├── Cast, CompoundLiteral, Conditional, Assignment
    ├── Sizeof, Alignof, GenericSelection
    └── Comma
```

### The Three-Way Ambiguity: `(type)` patterns

When the parser sees `(`, three things are possible:
1. **Parenthesized expression:** `(a + b)`
2. **Cast expression:** `(int)x`
3. **Compound literal:** `(int[]){1, 2, 3}`

Resolution algorithm:
- After `(`, try to parse as a type-name (using the typedef table)
- If type-name parse succeeds and `)` follows:
  - If the next token after `)` is `{` → **compound literal**
  - Else → **cast expression**
- If type-name parse fails → **parenthesized expression**

This requires speculative parsing (try type-name, backtrack if it fails) OR a lookahead heuristic (if token after `(` is a type keyword or typedef name, it's a type context).

### Expression Precedence (Pratt binding powers)

| Level | Operators | Assoc |
|-------|-----------|-------|
| 15 | `,` (comma) | Left |
| 14 | `= += -= *= /= %= <<= >>= &= ^= \|=` | Right |
| 13 | `? :` (ternary) | Right |
| 12 | `\|\|` | Left |
| 11 | `&&` | Left |
| 10 | `\|` | Left |
| 9 | `^` | Left |
| 8 | `&` | Left |
| 7 | `== !=` | Left |
| 6 | `< > <= >=` | Left |
| 5 | `<< >>` | Left |
| 4 | `+ -` | Left |
| 3 | `* / %` | Left |
| 2 | Unary prefix: `++ -- & * + - ~ ! sizeof _Alignof (cast)` | Right |
| 1 | Postfix: `() [] . -> ++ --` | Left |

---

## Acceptance Criteria

### Core
- [ ] Parse `int main() { return 0; }`
- [ ] Parse all statement types (if/else, while, do-while, for, switch/case, goto/label, return, break, continue, compound)
- [ ] Parse all declaration forms (variables, functions, typedefs, struct, union, enum)
- [ ] Parse complex declarators: `int (*(*fp)(int))[10]`
- [ ] Parse all expression operators with correct precedence
- [ ] Parse initializer lists with designators: `{ .x = 1, [0] = 2 }`
- [ ] Parse `_Generic`, `_Static_assert`, compound literals
- [ ] Typedef names resolve the declaration/expression ambiguity

### GNU Extension Tolerance
- [ ] `__attribute__((...))` is skipped (balanced paren consumption)
- [ ] `__extension__` is ignored (treated as no-op)
- [ ] `__restrict`, `__inline`, `__volatile__`, `__const` are treated as their standard equivalents
- [ ] `__asm__(...)` at declaration level is skipped (balanced parens)
- [ ] `__builtin_va_list` passes through as a type identifier
- [ ] `__typeof__(expr)` is parsed (GNU typeof)
- [ ] `__attribute__` on function declarations, parameters, struct members — all skipped cleanly

### Error Recovery & Real-World
- [ ] Syntax error in one function doesn't prevent parsing subsequent functions
- [ ] Can parse preprocessed `#include <stdio.h>` output without errors (token stream from Phase 2)
- [ ] Can parse a 50-line C program using most language features

---

## Claude Code Prompts

### Prompt 3.1 — AST type definitions

```
Create the forge_parser crate in the Forge workspace. Define the complete C17 AST
type hierarchy. DO NOT write the parser yet — just the types.

IMPORTANT DESIGN DECISION: Type specifiers are collected as a LIST, not a single enum.
In C, `unsigned long long int` is four separate type specifier tokens that combine.
The parser collects them into a Vec during parsing. Resolution to a concrete type
happens in Phase 4 (sema).

Create these files:

forge_parser/src/lib.rs — crate root, pub mod declarations
forge_parser/src/ast.rs — all AST node types
forge_parser/src/ast_ops.rs — operator enums (BinaryOp, UnaryOp, AssignOp)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
AST TYPES (define in ast.rs)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

TranslationUnit {
    declarations: Vec<ExternalDeclaration>,
    span: Span,
}

ExternalDeclaration — enum:
    FunctionDef(FunctionDef)
    Declaration(Declaration)

FunctionDef {
    specifiers: DeclSpecifiers,
    declarator: Declarator,
    // K&R-style old declarations between ) and { are NOT supported (deprecated)
    body: CompoundStmt,
    span: Span,
}

━━━ Declarations ━━━

Declaration {
    specifiers: DeclSpecifiers,
    init_declarators: Vec<InitDeclarator>,
    span: Span,
}

DeclSpecifiers {
    storage_class: Option<StorageClass>,
    type_specifiers: Vec<TypeSpecifierToken>,  // ← CRITICAL: this is a Vec
    type_qualifiers: Vec<TypeQualifier>,
    function_specifiers: Vec<FunctionSpecifier>,
    alignment: Option<AlignSpec>,
    // GNU: attributes collected here too
    attributes: Vec<GnuAttribute>,
    span: Span,
}

StorageClass — enum: Auto, Register, Static, Extern, Typedef, ThreadLocal

TypeSpecifierToken — enum:
    // Primitive keywords (collected into Vec, resolved by sema)
    Void, Char, Short, Int, Long, Float, Double,
    Signed, Unsigned, Bool, Complex,
    // Compound types (parsed inline)
    Struct(StructDef),
    Union(UnionDef),
    Enum(EnumDef),
    // Typedef reference
    TypedefName(String),
    // C11
    Atomic(Box<TypeName>),
    // GNU extension
    TypeofExpr(Box<Expr>),
    TypeofType(Box<TypeName>),

TypeQualifier — enum: Const, Volatile, Restrict, Atomic

FunctionSpecifier — enum: Inline, Noreturn

AlignSpec — enum:
    AlignAsType(TypeName),
    AlignAsExpr(Box<Expr>),

━━━ Declarators ━━━

InitDeclarator {
    declarator: Declarator,
    initializer: Option<Initializer>,
    span: Span,
}

Declarator {
    pointers: Vec<PointerQualifiers>,  // each level: * with optional qualifiers
    direct: DirectDeclarator,
    span: Span,
}

PointerQualifiers {
    qualifiers: Vec<TypeQualifier>,
}

DirectDeclarator — enum:
    Identifier(String, Span),
    Parenthesized(Box<Declarator>),
    Array {
        base: Box<DirectDeclarator>,
        size: ArraySize,
        qualifiers: Vec<TypeQualifier>,
        is_static: bool,
        span: Span,
    },
    Function {
        base: Box<DirectDeclarator>,
        params: Vec<ParamDecl>,
        is_variadic: bool,
        span: Span,
    },

ArraySize — enum:
    Unspecified,           // int arr[]
    Expr(Box<Expr>),       // int arr[10]
    VLA,                   // int arr[*] (VLA in prototype)

ParamDecl {
    specifiers: DeclSpecifiers,
    declarator: Option<Declarator>,  // None for abstract params like foo(int, int)
    span: Span,
}

━━━ Struct / Union / Enum ━━━

StructDef {
    kind: StructOrUnion,  // enum: Struct, Union
    name: Option<String>,
    members: Option<Vec<StructMember>>,  // None = forward declaration
    attributes: Vec<GnuAttribute>,
    span: Span,
}
// UnionDef is the same type as StructDef with kind = Union

StructOrUnion — enum: Struct, Union

StructMember {
    specifiers: DeclSpecifiers,
    declarators: Vec<StructDeclarator>,
    span: Span,
}

StructDeclarator {
    declarator: Option<Declarator>,  // None for anonymous bit-field
    bit_width: Option<Box<Expr>>,
    span: Span,
}

EnumDef {
    name: Option<String>,
    enumerators: Option<Vec<Enumerator>>,  // None = forward reference
    attributes: Vec<GnuAttribute>,
    span: Span,
}

Enumerator {
    name: String,
    value: Option<Box<Expr>>,
    span: Span,
}

━━━ Initializers ━━━

Initializer — enum:
    Expr(Box<Expr>),
    List { items: Vec<DesignatedInit>, span: Span },

DesignatedInit {
    designators: Vec<Designator>,
    initializer: Box<Initializer>,
    span: Span,
}

Designator — enum:
    Index(Box<Expr>),     // [expr]
    Field(String),         // .field

━━━ Statements ━━━

CompoundStmt {
    items: Vec<BlockItem>,
    span: Span,
}

BlockItem — enum:
    Declaration(Declaration),
    Statement(Stmt),

Stmt — enum:
    Compound(CompoundStmt),
    Expr(Option<Box<Expr>>),       // expression-stmt or empty-stmt (;)
    If {
        condition: Box<Expr>,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
        span: Span,
    },
    While { condition: Box<Expr>, body: Box<Stmt>, span: Span },
    DoWhile { body: Box<Stmt>, condition: Box<Expr>, span: Span },
    For {
        init: Option<ForInit>,
        condition: Option<Box<Expr>>,
        update: Option<Box<Expr>>,
        body: Box<Stmt>,
        span: Span,
    },
    Switch { expr: Box<Expr>, body: Box<Stmt>, span: Span },
    Case { value: Box<Expr>, body: Box<Stmt>, span: Span },
    Default { body: Box<Stmt>, span: Span },
    Return { value: Option<Box<Expr>>, span: Span },
    Break { span: Span },
    Continue { span: Span },
    Goto { label: String, span: Span },
    Label { name: String, stmt: Box<Stmt>, span: Span },
    // C11
    StaticAssert {
        condition: Box<Expr>,
        message: Option<String>,  // C23 makes message optional, include for forward compat
        span: Span,
    },

ForInit — enum:
    Declaration(Declaration),
    Expr(Box<Expr>),

━━━ Expressions ━━━

Expr — enum:
    // Literals
    IntLiteral { value: u64, suffix: Option<IntSuffix>, span: Span },
    FloatLiteral { value: f64, suffix: Option<FloatSuffix>, span: Span },
    CharLiteral { value: u32, prefix: Option<CharPrefix>, span: Span },
    StringLiteral { value: String, prefix: Option<StringPrefix>, span: Span },
    // Names
    Ident { name: String, span: Span },
    // Operators
    BinaryOp { op: BinaryOp, left: Box<Expr>, right: Box<Expr>, span: Span },
    UnaryOp { op: UnaryOp, operand: Box<Expr>, span: Span },
    PostfixOp { op: PostfixOp, operand: Box<Expr>, span: Span },
    // Ternary
    Conditional { condition: Box<Expr>, then_expr: Box<Expr>, else_expr: Box<Expr>, span: Span },
    // Assignment
    Assignment { op: AssignOp, target: Box<Expr>, value: Box<Expr>, span: Span },
    // Access
    FunctionCall { callee: Box<Expr>, args: Vec<Expr>, span: Span },
    MemberAccess { object: Box<Expr>, member: String, is_arrow: bool, span: Span },
    ArraySubscript { array: Box<Expr>, index: Box<Expr>, span: Span },
    // Type-related
    Cast { type_name: Box<TypeName>, expr: Box<Expr>, span: Span },
    SizeofExpr { expr: Box<Expr>, span: Span },
    SizeofType { type_name: Box<TypeName>, span: Span },
    AlignofType { type_name: Box<TypeName>, span: Span },
    CompoundLiteral { type_name: Box<TypeName>, initializer: Vec<DesignatedInit>, span: Span },
    // C11
    GenericSelection {
        controlling: Box<Expr>,
        associations: Vec<GenericAssociation>,
        span: Span,
    },
    // Comma
    Comma { exprs: Vec<Expr>, span: Span },

GenericAssociation {
    type_name: Option<TypeName>,  // None = default
    expr: Box<Expr>,
    span: Span,
}

━━━ Type Names (for sizeof, cast, compound literal) ━━━

TypeName {
    specifiers: DeclSpecifiers,
    abstract_declarator: Option<AbstractDeclarator>,
    span: Span,
}

AbstractDeclarator {
    pointers: Vec<PointerQualifiers>,
    direct: Option<DirectAbstractDeclarator>,
    span: Span,
}

DirectAbstractDeclarator — enum:
    Parenthesized(Box<AbstractDeclarator>),
    Array { base: Option<Box<DirectAbstractDeclarator>>, size: ArraySize, span: Span },
    Function { base: Option<Box<DirectAbstractDeclarator>>, params: Vec<ParamDecl>, is_variadic: bool, span: Span },

━━━ GNU Extensions ━━━

GnuAttribute {
    name: String,
    args: Option<Vec<GnuAttributeArg>>,  // None = no parens; Some(vec![]) = empty parens
    span: Span,
}

GnuAttributeArg — enum:
    Ident(String),
    Expr(Box<Expr>),
    // Could be more complex but this covers 95% of system headers

━━━ Operator enums (ast_ops.rs) ━━━

BinaryOp — enum:
    Add, Sub, Mul, Div, Mod,
    BitAnd, BitOr, BitXor,
    Shl, Shr,
    LogAnd, LogOr,
    Eq, Ne, Lt, Gt, Le, Ge,

UnaryOp — enum:
    PreIncrement, PreDecrement,
    AddrOf, Deref,
    Plus, Minus,
    BitNot, LogNot,

PostfixOp — enum:
    PostIncrement, PostDecrement,

AssignOp — enum:
    Assign,
    AddAssign, SubAssign, MulAssign, DivAssign, ModAssign,
    BitAndAssign, BitOrAssign, BitXorAssign,
    ShlAssign, ShrAssign,

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

All AST node types should derive Debug and Clone.
Every node that represents a source construct should have a `span: Span` field.

Write a few tests that manually construct AST nodes to verify the types compile:
- Build a simple FunctionDef node for `int main() { return 0; }`
- Build a Declaration node for `unsigned long long x = 42;` 
  (verify that type_specifiers is a Vec containing [Unsigned, Long, Long, Int])
- Build an Expr node for `a + b * c` with correct nesting
- Build a StructDef with members and a bit-field

Add forge_parser to workspace Cargo.toml. Add forge_lexer and forge_diagnostics 
as dependencies.
```

### Prompt 3.2 — Parser infrastructure + Pratt expression parser

```
Implement the parser infrastructure and expression parser in forge_parser.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Parser struct and helpers
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create forge_parser/src/parser.rs with:

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    typedefs: Vec<HashSet<String>>,  // stack of scopes, last = current
    diagnostics: Vec<Diagnostic>,
}

Core methods:
- peek() -> &Token                    // look at current token without consuming
- peek_ahead(n: usize) -> &Token     // look n tokens ahead
- advance() -> Token                  // consume and return current token
- expect(kind: TokenKind) -> Result<Token, ()>  // consume if match, diagnostic if not
- at(kind: TokenKind) -> bool         // check without consuming
- eat(kind: TokenKind) -> Option<Token>  // consume if match, None if not
- at_eof() -> bool
- span_from(start: Span) -> Span      // merge start span with previous token's span

Typedef scope methods:
- push_scope()           // push new HashSet onto typedefs stack
- pop_scope()            // pop the top scope
- add_typedef(name)      // insert into current (top) scope
- is_typedef(name) -> bool  // check ALL scopes (walk stack top to bottom)

pub fn parse(tokens: Vec<Token>) -> Result<TranslationUnit, Vec<Diagnostic>>
  — entry point, creates Parser, calls parse_translation_unit()

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Pratt expression parser
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create forge_parser/src/expr.rs with expression parsing methods on Parser.

Core functions:
- parse_expr() → Expr               // full expression including comma
- parse_assignment_expr() → Expr     // assignment and above (no comma)
- parse_conditional_expr() → Expr    // ternary and above
- parse_pratt(min_bp: u8) → Expr     // the core Pratt loop

The Pratt loop:
1. Parse a prefix/primary expression (nud)
2. Loop: peek at next token
   - Get its infix/postfix binding power
   - If binding power <= min_bp, stop
   - Consume the operator and parse the right side
   - Build the appropriate AST node
3. Return the expression

Prefix/primary (nud) parsing — parse_prefix():
- IntegerLiteral → Expr::IntLiteral
- FloatLiteral → Expr::FloatLiteral  
- CharLiteral → Expr::CharLiteral
- StringLiteral → Expr::StringLiteral (handle adjacent string concatenation!)
- Identifier → Expr::Ident
- LeftParen → THREE-WAY AMBIGUITY (see below)
- Prefix operators: ++, --, &, *, +, -, ~, !
- sizeof → sizeof(type) or sizeof expr (if followed by '(' check if type-name)
- _Alignof → _Alignof(type)
- _Generic → parse generic selection

THREE-WAY `(` AMBIGUITY — when parse_prefix sees LeftParen:
1. Save current position (for backtracking)
2. After `(`, check if the next token starts a type-name:
   - Is it a type keyword (void, char, int, float, double, short, long,
     signed, unsigned, _Bool, _Complex, _Atomic, struct, union, enum)?
   - Is it a type qualifier (const, volatile, restrict)?
   - Is it a known typedef name (check is_typedef())?
3. If YES → tentatively parse type-name, expect `)`
   - If next token is `{` → COMPOUND LITERAL: parse initializer list
   - Else → CAST: parse the operand as a unary expression
4. If NO → PARENTHESIZED EXPRESSION: parse expression, expect `)`

If the tentative type-name parse fails (e.g., it looked like a typedef but
the grammar didn't work out), backtrack to saved position and parse as
parenthesized expression.

Infix/postfix (led) parsing:
- Binary operators: +, -, *, /, %, <<, >>, <, >, <=, >=, ==, !=, &, ^, |, &&, ||
- Ternary: ? ... :  (right-associative: use lower right binding power)
- Assignment: =, +=, -=, etc. (right-associative)
- Comma: , (lowest precedence)
- Postfix: 
  - LeftParen → function call: parse argument list
  - LeftBracket → array subscript: parse expression, expect ]
  - Dot → member access
  - Arrow → pointer member access
  - PlusPlus → post-increment
  - MinusMinus → post-decrement

String literal concatenation:
When you see a StringLiteral, peek ahead — if the next token is also a
StringLiteral, concatenate them. This is required by C17 (adjacent string
literals are merged in translation phase 6).

Binding powers (as u8 pairs for left/right):
  Comma:       (2, 3)
  Assignment:  (4, 3)   ← right-assoc: right bp LOWER than left
  Ternary:     (6, 5)   ← right-assoc
  LogOr:       (8, 9)
  LogAnd:      (10, 11)
  BitOr:       (12, 13)
  BitXor:      (14, 15)
  BitAnd:      (16, 17)
  Equality:    (18, 19)
  Relational:  (20, 21)
  Shift:       (22, 23)
  Additive:    (24, 25)
  Multiplicative: (26, 27)
  Prefix:      (_, 29)
  Postfix:     (31, _)

Write THOROUGH tests:
- Precedence: `1 + 2 * 3` → Add(1, Mul(2, 3))
- Associativity: `a = b = c` → Assign(a, Assign(b, c))
- Associativity: `a - b - c` → Sub(Sub(a, b), c)
- Ternary: `a ? b : c ? d : e` → Cond(a, b, Cond(c, d, e))
- Unary: `-!x` → Minus(LogNot(x))
- Postfix chain: `a[0].b->c++` → correct nesting
- Function call: `f(a, b, c)` → FunctionCall with 3 args
- Cast: `(int)x` → Cast(int, x) — requires setting up "int" as known type
- Compound literal: `(int[]){1, 2}` → CompoundLiteral
- sizeof: `sizeof(int)` vs `sizeof x` — both forms
- String concatenation: `"hello" " " "world"` → single StringLiteral "hello world"
- Comma: `a, b, c` → Comma([a, b, c])
- Complex: `*p++ = f(a + b, c)` → correct tree
```

### Prompt 3.3 — Declaration specifiers + simple declarators + typedef tracking

```
Implement declaration specifier parsing, simple declarators, and typedef tracking.

This prompt handles the FIRST HALF of C declaration parsing. Complex declarators
(arrays, function pointers), struct/enum definitions, and initializers come in 3.4.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Declaration specifiers
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_declaration_specifiers() → DeclSpecifiers

Loop consuming tokens that are valid specifiers:

Storage class keywords: auto, register, static, extern, typedef, _Thread_local
  → Only one storage class per declaration (error on duplicates)

Type specifier keywords: void, char, short, int, long, float, double, 
  signed, unsigned, _Bool, _Complex
  → Collect into Vec<TypeSpecifierToken>
  → Multiple are allowed: `unsigned long long int` = 4 entries
  → Some combinations are invalid (e.g., `float double`) — we detect this in
    sema, NOT here. The parser just collects.

Type qualifiers: const, volatile, restrict, _Atomic
  → Collect into Vec, duplicates allowed (redundant but legal)

Function specifiers: inline, _Noreturn

Known typedef names: if an identifier is in the typedef set AND we haven't
  already seen a type specifier that would make this a redeclaration, treat
  it as TypeSpecifierToken::TypedefName(name).
  EDGE CASE: `typedef int T; { T T; }` — the second T is a variable named T
  using type T. Once we've seen a type specifier, further identifiers are
  declarator names, not typedefs.

_Alignas(type) or _Alignas(expr) → AlignSpec

struct/union/enum → handled in Prompt 3.4 (for now, just call a placeholder
  parse_struct_or_union_specifier() that you'll implement next prompt)

__attribute__((...)) → skip for now (call skip_gnu_attribute() placeholder)

The specifier loop stops when the next token is NOT a specifier keyword,
typedef name, or qualifier.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Simple declarators
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_declarator() → Declarator

A declarator is: optional pointer prefix(es) + direct-declarator

Pointer prefix: * optionally followed by type qualifiers (const, volatile, etc.)
  Can be chained: `int **const *volatile x;` = three pointer levels
  Parse as Vec<PointerQualifiers>

Direct declarator (simple version for now):
  - Identifier → DirectDeclarator::Identifier
  - ( declarator ) → DirectDeclarator::Parenthesized (for grouping, e.g., (*fp))

Suffixes on direct-declarator will be added in Prompt 3.4:
  - [size] for arrays
  - (params) for functions
  For now, implement these as stubs or TODOs.

parse_abstract_declarator() → AbstractDeclarator
  Same as declarator but no identifier (used in type-names for sizeof, cast, etc.)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Declaration and init-declarator list
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_declaration() → Declaration

1. Parse declaration specifiers
2. If followed by `;` → declaration with no declarators (e.g., `struct foo;`)
3. Otherwise parse init-declarator-list:
   - parse_declarator()
   - If followed by `=` → parse_initializer() (simple expr form for now;
     brace-enclosed initializer lists come in 3.4)
   - If followed by `,` → continue to next init-declarator
   - Expect `;` at end

4. CRITICAL — typedef tracking:
   If the specifiers included StorageClass::Typedef, then for each declarator
   in the init-declarator list, extract the declared name and call add_typedef().
   Example: `typedef unsigned long size_t;` → add "size_t" to typedef set
   Example: `typedef int (*handler_t)(int);` → add "handler_t" to typedef set
   The name is found by walking the declarator to find the Identifier node.

parse_type_name() → TypeName
  Used by sizeof(type), cast, compound literal, _Alignas(type), _Atomic(type).
  It's like a declaration but with an abstract declarator (no name).
  1. Parse declaration specifiers
  2. Optionally parse abstract declarator

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — The declaration/expression ambiguity
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

At block scope, when parsing a block-item, the parser must decide:
is this a declaration or a statement?

is_start_of_declaration() → bool:
  Peek at the current token:
  - Type keywords (void, char, int, float, etc.) → declaration
  - Storage class keywords (static, extern, typedef, etc.) → declaration
  - Type qualifiers at start (const, volatile) → declaration
  - _Static_assert → special declaration
  - struct, union, enum → declaration
  - Known typedef name → declaration (THIS is why typedef tracking matters)
  - __attribute__ → declaration (followed by declaration specifiers)
  - Anything else → statement (expression statement, if, while, etc.)

This function is called in parse_block_item() to choose between
parse_declaration() and parse_statement().

Write tests:
- Parse `int x;` → declaration with type_specifiers=[Int], one declarator "x"
- Parse `unsigned long long x;` → type_specifiers=[Unsigned, Long, Long], declarator "x"
- Parse `const int *p;` → qualifier=Const, specifier=Int, one pointer level
- Parse `int **const *x;` → three pointer levels, middle has Const
- Parse `int x = 5;` → declarator "x" with initializer IntLiteral(5)
- Parse `int x, y, *z;` → three init-declarators
- Parse `typedef int MyInt;` → "MyInt" added to typedef set
- Parse `MyInt x;` after typedef → resolves MyInt as type specifier
- Parse `int (*fp);` → parenthesized declarator
- Parse `int x; x * y;` → first is declaration, second is expression (multiply)
- Parse `typedef int T; T * x;` → second is declaration (pointer to T), not multiply
```

### Prompt 3.4 — Complex declarators, struct/enum, initializers

```
Implement complex declarators (array, function), struct/union/enum definitions,
and brace-enclosed initializer lists.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Declarator suffixes (array and function)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Extend parse_direct_declarator() with suffix parsing.

After parsing the identifier or parenthesized declarator, loop checking for:

Array suffix: [
  - `[]` → ArraySize::Unspecified
  - `[expr]` → ArraySize::Expr
  - `[*]` → ArraySize::VLA (only in function prototypes)
  - `[static expr]` → is_static=true, ArraySize::Expr
  - `[const expr]` → qualifiers, ArraySize::Expr
  Build DirectDeclarator::Array wrapping the current direct-declarator as base.

Function suffix: (
  - Parse parameter-type-list (see Section 2)
  - Or empty `()` → no params, not variadic (old-style, we treat as no params)
  - `(void)` → explicitly no params
  Build DirectDeclarator::Function wrapping current as base.

This is recursive in nature: `int (*fp)(int, int)` is:
  1. Pointer declarator wrapping parenthesized declarator `fp`
  2. Function suffix with (int, int) parameters
The parsing naturally handles this via the base-then-suffix approach.

Also extend parse_direct_abstract_declarator() with the same suffix logic
(but no identifier, for type-names in casts/sizeof).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Parameter list
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_parameter_list() → (Vec<ParamDecl>, bool)  // params + is_variadic

- Parse comma-separated parameter declarations
- Each param: declaration-specifiers + optional declarator (or abstract-declarator)
  - `int x` → specifiers=[Int], declarator=Identifier("x")
  - `int *` → specifiers=[Int], abstract declarator with pointer
  - `int` → specifiers=[Int], no declarator
- If `...` appears after the last comma → is_variadic = true
- `(void)` with no declarator → zero params, not variadic
- `()` → zero params (ambiguous with old-style, but we treat as no params)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Struct and union definitions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_struct_or_union_specifier() → TypeSpecifierToken

After `struct` or `union`:
1. Optional name (identifier)
2. If followed by `{` → parse member list:
   - Each member: declaration-specifiers + struct-declarator-list ;
   - struct-declarator: optional declarator + optional `: bit-width`
     - `int x;` → normal member
     - `int x : 3;` → bit-field
     - `int : 5;` → anonymous bit-field (no declarator, just width)
   - Loop until `}`
3. If no `{` → forward declaration: `struct foo` (just a name, no members)

Both forms produce TypeSpecifierToken::Struct(StructDef) or ::Union(...)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Enum definitions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_enum_specifier() → TypeSpecifierToken

After `enum`:
1. Optional name
2. If followed by `{` → parse enumerator list:
   - Comma-separated: name [= constant-expr]
   - Trailing comma before `}` is allowed
   - Each enumerator creates Enumerator { name, value }
3. If no `{` → forward reference

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Initializer lists (brace-enclosed)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Extend parse_initializer() to handle brace-enclosed lists:

parse_initializer() → Initializer:
  If current token is `{` → parse_initializer_list()
  Else → Initializer::Expr(parse_assignment_expr())

parse_initializer_list() → Initializer::List:
  After `{`:
  - Parse comma-separated designated-initializers
  - Each: optional designator-list + initializer
  - Designators: `.field` or `[index]`, can be chained: `.pos[0].x`
  - Trailing comma before `}` is allowed (C99+)
  - Nested: `{ {1, 2}, {3, 4} }` is valid (nested brace-enclosed lists)

Write tests:
- Simple array: `int a[3];`
- Multi-dimensional: `int a[2][3];`
- Function declaration: `int f(int a, char *b);`
- Function pointer: `int (*fp)(int, int);`
- Array of function pointers: `int (*fps[10])(void);`
- Complex: `int (*(*fp)(int))[10];` (pointer to function returning pointer to array)
- Struct with members: `struct Point { int x; int y; };`
- Struct with bit-fields: `struct Flags { unsigned int a : 1; unsigned int b : 3; };`
- Anonymous bit-field: `struct { int : 4; int x : 4; };`
- Enum: `enum Color { RED, GREEN = 5, BLUE };`
- Initializer list: `int a[] = {1, 2, 3};`
- Designated init: `struct Point p = { .x = 1, .y = 2 };`
- Nested init: `int m[2][2] = { {1, 2}, {3, 4} };`
- Array designated: `int a[10] = { [5] = 50, [9] = 90 };`
- VLA parameter: `void f(int n, int arr[n]);`
- `(void)` parameter: `int f(void);`
- Variadic: `int printf(const char *fmt, ...);`
```

### Prompt 3.5 — Statements, top-level parsing, error recovery

```
Implement statement parsing, top-level translation unit parsing, and error recovery.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Statement parsing
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_statement() → Stmt:

Dispatch based on current token:
  `{`         → parse_compound_statement()
  `if`        → parse_if_statement()
  `while`     → parse_while_statement()
  `do`        → parse_do_while_statement()
  `for`       → parse_for_statement()
  `switch`    → parse_switch_statement()
  `case`      → parse_case_statement()
  `default`   → parse_default_statement()
  `return`    → parse_return_statement()
  `break`     → Stmt::Break, expect `;`
  `continue`  → Stmt::Continue, expect `;`
  `goto`      → Stmt::Goto(identifier), expect `;`
  `;`         → Stmt::Expr(None) — empty statement
  identifier followed by `:` → Stmt::Label { name, stmt }
  otherwise   → expression-statement: parse_expr(), expect `;`

Compound statement:
  `{` block-item* `}`
  push_scope() before, pop_scope() after (typedef scoping!)
  block-item: call is_start_of_declaration() → Declaration or Statement

If statement:
  `if` `(` expr `)` stmt [else stmt]
  Dangling else: else binds to the innermost if (natural with recursive descent)

For statement:
  `for` `(` init `;` condition `;` update `)` body
  init can be:
  - A declaration (check is_start_of_declaration())
  - An expression
  - Empty (just `;`)
  For declarations in init: `for (int i = 0; ...)` — the scope of `i` is the for body.
  Push scope before parsing init, pop after body.

Switch / case / default:
  `switch` `(` expr `)` stmt  (stmt is usually a compound)
  `case` constant-expr `:` stmt
  `default` `:` stmt

Do-while:
  `do` stmt `while` `(` expr `)` `;`

_Static_assert:
  `_Static_assert` `(` constant-expr `,` string-literal `)` `;`
  (C23 makes the string optional — support both forms)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Top-level parsing
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_translation_unit() → TranslationUnit:

Loop until EOF:
  parse_external_declaration() → ExternalDeclaration

parse_external_declaration():
  1. Parse declaration specifiers
  2. If followed by `;` → declaration with no declarator (e.g., `struct foo;`)
  3. Parse first declarator
  4. DECISION POINT — function definition or declaration?
     - If the declarator is a function declarator AND next token is `{`
       → FunctionDef: parse compound statement as body
     - Otherwise → Declaration: parse rest of init-declarator list, expect `;`

  This means: `int main() { return 0; }` is:
    specifiers=[Int], declarator=Function("main", []), body={return 0}

  And: `int foo(int x);` is:
    specifiers=[Int], init_declarators=[Function("foo", [int x])], semicolon

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Error recovery
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

When an unexpected token is encountered:
1. Emit a diagnostic with the token's span and a descriptive message
   e.g., "expected ';' after declaration" or "unexpected token 'foo' in expression"

2. Synchronize by skipping tokens until a recovery point:
   synchronize() method:
   - Skip tokens until one of:
     - `;` (consume it — end of statement/declaration)
     - `}` (do NOT consume — it might close an enclosing block)
     - `{` (do NOT consume — it might start a function body)
     - A token at start of line that looks like a new declaration
       (type keyword, storage class keyword)
     - EOF

3. After synchronizing, return a synthetic "error" node or simply continue
   parsing from the recovery point.

4. Collect ALL errors — don't stop at the first one. Return all diagnostics
   at the end.

5. Set an error flag so the final Result reflects that errors occurred,
   even though the AST was (partially) built.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Write tests:
- `int main() { return 0; }` → FunctionDef, compound with return
- `void f() { if (x) y(); else z(); }` → if/else
- `void f() { while (1) { break; } }` → while + break
- `void f() { do { x++; } while (x < 10); }` → do-while
- `void f() { for (int i = 0; i < n; i++) sum += i; }` → for with decl init
- `void f() { for (;;) break; }` → infinite for
- `void f() { switch (x) { case 1: a(); break; default: b(); } }` → switch
- `void f() { goto end; end: return; }` → goto + label
- Multiple functions in one file → TranslationUnit with multiple ExternalDeclarations
- Nested blocks with local declarations and typedef scoping
- Empty statement: `void f() { ; }` → Stmt::Expr(None)
- _Static_assert(sizeof(int) == 4, "oops");
- Error recovery: `int x = ;` → error, but parsing continues to next declaration
- Error recovery: `int f() { int x = 5 int y = 6; }` → missing `;`, recovers
- Multiple errors in one file → all reported
```

### Prompt 3.6 — GNU extension tolerance + AST printer

```
Add GNU extension tolerance so the parser can handle preprocessed system header
output, and implement an AST pretty-printer.

MOTIVATION: After `#include <stdio.h>`, the preprocessed token stream contains
GNU-specific syntax that the parser must handle. Without this, `forge check` on
any file that includes a system header will fail. This is the parser equivalent
of the predefined macros we added in Prompt 2.5.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — __attribute__ handling
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

__attribute__((...)) can appear in MANY positions in declarations:
  - After type specifiers: `int __attribute__((packed)) x;`
  - After declarators: `int x __attribute__((aligned(16)));`
  - On function declarations: `void f(void) __attribute__((noreturn));`
  - On struct/union/enum: `struct __attribute__((packed)) S { ... };`
  - On function parameters: `void f(int x __attribute__((unused)));`

Implementation — skip_gnu_attributes():
  When you see the identifier `__attribute__` (or `__attribute`):
  1. Expect `(`
  2. Expect `(` (yes, double parens — `__attribute__((...))`
  3. Count balanced parens, consuming everything until the matching `))`
  4. Store the consumed attributes in the GnuAttribute list on DeclSpecifiers
     (or discard them — for Phase 3, we just need to not choke on them)
  5. Return

Call skip_gnu_attributes() at these points:
  - In parse_declaration_specifiers() loop (when you see __attribute__)
  - After parse_declarator() (attributes can follow the declarator)
  - After the closing `)` of a function parameter list
  - After struct/union/enum keyword
  - After enum enumerators

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Other GNU extensions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

__extension__:
  When seen, simply consume it and continue parsing the next declaration/expression.
  It's a GCC directive meaning "suppress warnings for the following extension."
  Treat as a no-op prefix.

__restrict, __restrict__:
  Treat as equivalent to `restrict` (TypeQualifier::Restrict).

__inline, __inline__:
  Treat as equivalent to `inline` (FunctionSpecifier::Inline).

__volatile, __volatile__:
  Treat as equivalent to `volatile` (TypeQualifier::Volatile).

__const, __const__:
  Treat as equivalent to `const` (TypeQualifier::Const).

__signed, __signed__:
  Treat as equivalent to `signed` (TypeSpecifierToken::Signed).

__typeof__(expr) and __typeof(expr):
  Parse the expression inside parens, produce TypeSpecifierToken::TypeofExpr.
  Also handle `typeof(type-name)` form → TypeSpecifierToken::TypeofType.

__builtin_va_list:
  Treat as a typedef name. Add it to the initial typedef set during parser setup
  (alongside any other compiler builtin types).

__asm__ or asm at declaration level:
  After a function declarator, `__asm__("symbol_name")` specifies the assembly
  name. Consume the balanced parens and discard.
  Also handle `__asm__` in struct members (bitfield assembly names).

__builtin_offsetof(type, member):
  Parse as a function-call-like expression. The parser doesn't need to understand
  it semantically — just parse the balanced parens.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — AST pretty-printer
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Implement a tree-style AST dump for debugging and testing.

Create forge_parser/src/printer.rs with:
  pub fn print_ast(unit: &TranslationUnit) -> String

Output format (indented tree):
```
TranslationUnit
  FunctionDef "main" → [Int]
    Params: (void)
    CompoundStmt
      ReturnStmt
        IntLiteral 0
```

This should show:
- Type specifiers as a list: `[Unsigned, Long, Long]`
- Declarator structure: pointer levels, array sizes, function params
- Expression trees with operator names
- Indentation for nesting depth

Implement Display for the main AST types, or a dedicated Printer struct with
an indent level.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

A. GNU attribute tests:
   - `int x __attribute__((aligned(16)));` → parses, attribute stored or skipped
   - `void f(void) __attribute__((noreturn));` → parses
   - `struct __attribute__((packed)) S { int x; };` → parses
   - `__attribute__((visibility("default"))) int x;` → parses

B. GNU keyword tests:
   - `__inline__ int f(void) { return 0; }` → FunctionSpecifier::Inline
   - `int __const x = 5;` → TypeQualifier::Const
   - `int * __restrict p;` → TypeQualifier::Restrict
   - `__extension__ typedef ...` → skips __extension__, parses typedef

C. __typeof__ tests:
   - `__typeof__(x) y;` → TypeofExpr
   - `__typeof__(int *) p;` → TypeofType

D. __asm__ tests:
   - `extern int foo __asm__("_foo");` → skips asm, parses declaration

E. System header smoke test (THE BIG TEST):
   - Take the preprocessed output of `#include <stdio.h>` (from Phase 2)
   - Feed it to the parser
   - Assert: zero parse errors
   - This may require iterative fixing — some constructs in system headers
     may not be covered above. Common additions needed:
     - `__builtin_va_list` as a built-in typedef
     - `__attribute__` in unexpected positions
     - `_Pragma` tokens (should already be handled by preprocessor)

F. AST printer:
   - Parse `int main() { return 0; }` and verify the printed tree

Run cargo test --all, cargo clippy, cargo fmt.
```

### Prompt 3.7 — Integration, driver wiring, and validation

```
Integrate the parser into the Forge driver and run comprehensive validation.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Driver integration
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Update forge_driver:
1. After preprocessing, feed the token stream to the parser
2. `forge check file.c` now lexes → preprocesses → parses, reports all diagnostics
3. Add `forge parse file.c` subcommand that outputs the AST dump (from the printer)
4. `forge -E` still works as before (preprocess only, no parse)

Propagate parser diagnostics to the main diagnostic output alongside
preprocessor diagnostics.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Lit tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create test files in tests/lit/parser/:

tests/lit/parser/declarations.c — all declaration forms:
  Variables, pointers, arrays, function declarations, typedef, extern, static

tests/lit/parser/expressions.c — expression precedence and forms:
  Arithmetic, comparison, logical, bitwise, ternary, assignment, function call,
  array subscript, member access, cast, sizeof, compound literal

tests/lit/parser/statements.c — all statement types:
  if, while, for, switch, goto, return, break, continue, labeled, compound

tests/lit/parser/structs.c — struct/union/enum:
  Definition, forward declaration, bit-fields, nested structs, anonymous

tests/lit/parser/initializers.c — initializer lists:
  Simple, designated (field and array), nested

tests/lit/parser/complex.c — a complete 50-line C program using most features

tests/lit/parser/errors.c — syntax errors with expected error messages:
  // ERROR: expected ';'  
  // ERROR: unexpected token

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Completeness matrix
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Verify a test exists and passes for each:

DECLARATIONS:
| Feature                                    | Test? | Pass? |
|--------------------------------------------|-------|-------|
| int x;                                     |       |       |
| int x = 5;                                 |       |       |
| int x, y, *z;                              |       |       |
| unsigned long long x;                      |       |       |
| const int *p;                              |       |       |
| int *const p;                              |       |       |
| typedef int MyInt;                         |       |       |
| MyInt x; (typedef usage)                   |       |       |
| static int x;                              |       |       |
| extern int x;                              |       |       |
| int arr[10];                               |       |       |
| int arr[];                                 |       |       |
| int f(int a, char *b);                     |       |       |
| int f(void);                               |       |       |
| int f(int, ...);                           |       |       |
| int (*fp)(int);                            |       |       |
| int (*(*fp)(int))[10];                     |       |       |
| struct Point { int x; int y; };            |       |       |
| struct Point p;                            |       |       |
| struct { int x; } anon;                    |       |       |
| union { int i; float f; };                 |       |       |
| enum Color { RED, GREEN, BLUE };           |       |       |
| Bit-fields                                 |       |       |
| _Static_assert(...)                        |       |       |
| _Alignas(16) int x;                       |       |       |

EXPRESSIONS:
| Feature                                    | Test? | Pass? |
|--------------------------------------------|-------|-------|
| Arithmetic precedence (+ * mixing)          |       |       |
| All binary operators                       |       |       |
| All unary operators                        |       |       |
| Ternary (right-assoc)                      |       |       |
| Assignment (right-assoc)                   |       |       |
| Function call                              |       |       |
| Array subscript                            |       |       |
| Member access (. and ->)                   |       |       |
| Post-increment/decrement                   |       |       |
| Cast expression                            |       |       |
| sizeof(expr) and sizeof(type)              |       |       |
| Compound literal                           |       |       |
| _Generic selection                         |       |       |
| String literal concatenation               |       |       |
| Comma expression                           |       |       |

STATEMENTS:
| Feature                                    | Test? | Pass? |
|--------------------------------------------|-------|-------|
| Compound statement (block)                 |       |       |
| if / else                                  |       |       |
| while                                      |       |       |
| do-while                                   |       |       |
| for (expr init)                            |       |       |
| for (decl init)                            |       |       |
| switch / case / default                    |       |       |
| goto / label                               |       |       |
| return with and without value              |       |       |
| break / continue                           |       |       |
| Empty statement                            |       |       |

GNU EXTENSIONS:
| Feature                                    | Test? | Pass? |
|--------------------------------------------|-------|-------|
| __attribute__((...))                        |       |       |
| __extension__                               |       |       |
| __restrict / __inline / __volatile__        |       |       |
| __typeof__(expr)                            |       |       |
| __asm__(...)                                |       |       |
| __builtin_va_list                           |       |       |
| Preprocessed stdio.h parses without errors |       |       |

ERROR RECOVERY:
| Feature                                    | Test? | Pass? |
|--------------------------------------------|-------|-------|
| Missing semicolon → recovers               |       |       |
| Unexpected token → skips to next stmt       |       |       |
| Error in one function → next function ok    |       |       |
| Multiple errors collected                  |       |       |

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Edge case stress tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Write these as tests (no panics!):
1. Empty file → empty TranslationUnit
2. File with only comments → empty TranslationUnit
3. 50 nested blocks: `{ { { ... } } }` → parses without stack overflow
4. Very long expression: `a+a+a+...+a` (100 terms) → parses
5. Struct with 100 members
6. Function with 50 parameters
7. 20 levels of pointer indirection: `int **...*x;`
8. Deeply nested initializer: `{{{{{1}}}}}` (5 levels)
9. Declaration vs expression ambiguity: typedef then use in various contexts
10. Empty function body: `void f() {}`

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Real-world parse test
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create a test C file (tests/lit/parser/mini_program.c) that exercises most features:

```c
#include <stdio.h>
#include <stdlib.h>

typedef unsigned long size_t_alias;

struct Point {
    int x;
    int y;
};

enum Direction { UP, DOWN, LEFT = 10, RIGHT };

static int add(int a, int b) {
    return a + b;
}

int (*get_op(char op))(int, int) {
    switch (op) {
    case '+': return add;
    default:  return NULL;
    }
}

int main(int argc, char *argv[]) {
    struct Point p = { .x = 1, .y = 2 };
    int arr[] = {1, 2, 3, 4, 5};
    size_t_alias n = sizeof(arr) / sizeof(arr[0]);
    
    for (int i = 0; i < (int)n; i++) {
        if (arr[i] > 3) {
            printf("big: %d\n", arr[i]);
        }
    }
    
    int (*op)(int, int) = get_op('+');
    int result = op ? op(p.x, p.y) : -1;
    return result == 3 ? EXIT_SUCCESS : EXIT_FAILURE;
}
```

Feed this through: lex → preprocess → parse. Assert zero errors.
If it fails, fix the parser and re-run.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 6 — Final verification
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Run:
  cargo test --all → all tests pass (Phase 0-3 combined)
  cargo clippy --all-targets --all-features -- -D warnings → clean
  cargo fmt --all -- --check → clean

Report total test count and verdict: is forge_parser ready for Phase 4?
```

---

## Pitfalls & Debugging Tips

### "typedef int T; T * x;" must parse as a pointer declaration, not multiplication
This is the single most important test case. If your typedef tracking is broken, 
ALL code using typedef types will be misparsed. Test this early and test it often.

### "Declarators are read from inside out"
`int (*fp)(int)` → fp is a: pointer to → function taking int → returning int.
The `(*fp)` part is the declarator (pointer + name), and `(int)` is a function suffix.
Parse the innermost part first (via parenthesized declarator), then the suffixes.

### "Specifier order doesn't matter"
`long unsigned int` and `unsigned int long` are the same type in C.
Collect specifiers in a Vec and resolve the combination in sema (Phase 4).

### "Cast ambiguity requires typedef tracking"
`(x)(y)` is a function call if x is a variable, but a cast if x is a typedef.
Without the typedef table, the parser cannot decide. This is why typedef tracking
must work perfectly before expression parsing is complete.

### "__attribute__ can appear almost anywhere"
System headers put `__attribute__((visibility("default")))` before declarations,
after declarators, on function parameters, on struct fields, after enum values.
The safest approach: whenever you see `__attribute__`, call skip_gnu_attributes()
regardless of context.

### "Error recovery is about finding the next good starting point"
Don't try to understand what went wrong — just find the next `;` or `}` or 
type keyword at start of line, and restart parsing from there.

---

## Notes

- **Don't do type checking here.** `unsigned float x;` is grammatically valid — the parser
  accepts it, sema (Phase 4) rejects it.
- **Don't resolve types.** `[Unsigned, Long, Long]` stays as a Vec in the AST. Sema resolves it
  to a concrete type.
- **Don't evaluate constant expressions.** `int arr[2+3]` stores `Add(2, 3)` as the size,
  not `5`. Sema evaluates it.
- **K&R-style function definitions are NOT supported.** (`int f(x) int x; { ... }`)
  They've been deprecated since C89.
- **GNU statement expressions (`({...})`) are out of scope.** If encountered, emit a diagnostic.