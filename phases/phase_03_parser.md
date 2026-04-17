# Phase 3 — Parser & AST (Final)

**Depends on:** Phase 2 (Preprocessor) ✅ COMPLETE (536 tests, 38ms stdio.h)
**Unlocks:** Phase 4 (Semantic Analysis)
**Estimated duration:** 14–22 days (7 prompts)

---

## Goal

Build a hand-written recursive descent parser that consumes preprocessed tokens and produces a complete C17 AST. The parser must:

1. Handle the full C17 grammar — all declarations, statements, and expressions
2. Tolerate GNU extensions in system header output (`__attribute__`, `__extension__`, etc.)
3. Recover from errors — a syntax error in one function must not prevent parsing the rest
4. Track typedef names — the single most important correctness requirement

After this phase, `forge check file.c` runs the full pipeline: lex → preprocess → parse, and reports all diagnostics. `forge parse file.c` dumps the AST tree.

---

## Key Design Decisions

### 1. Type specifiers: collected as a Vec, resolved by sema (Phase 4)

C allows multiple type specifier keywords that combine: `unsigned long long int` is four separate tokens. The order doesn't matter (`long unsigned int long` is the same type). The parser collects all specifier tokens into a `Vec<TypeSpecifierToken>`. Phase 4 resolves the combination into a concrete type and rejects invalid combinations like `float double`.

**Rationale:** Trying to resolve during parsing leads to complex state machines and bad error messages. Deferring to sema keeps the parser simple and gives better diagnostics ("conflicting type specifiers: 'float' and 'double'" vs "unexpected token 'double'").

### 2. Start with `Box<>`, not arena allocation

Arena allocators (`bumpalo`, `typed-arena`) reduce allocation overhead but add lifetime complexity. Since the AST design will evolve through Phase 4–5 as we discover needs, start with `Box<>` for all recursive AST nodes. Profile in Phase 11 and migrate to arena only if allocation shows up as a bottleneck.

### 3. GNU extension tolerance is NOT optional

After `#include <stdio.h>`, the preprocessed token stream contains `__attribute__((...))`, `__extension__`, `__restrict`, `__inline__`, `__asm__(...)`, `__typeof__`, `__builtin_va_list`, and more. The parser must handle every one of these — at minimum by skipping balanced parens for attribute/asm, and by treating keyword variants as their standard equivalents. Without this, `forge check` on any real C file fails.

### 4. Typedef tracking is the #1 correctness issue

`T * x;` — is this a pointer declaration or a multiplication expression? The parser can only decide by knowing whether `T` was previously declared as a typedef. A wrong answer here means **silent misparsing** of every line that uses a typedef type. The parser must:
- Maintain a scoped set of typedef names (stack of `HashSet<String>`)
- Update it as `typedef` declarations are parsed
- Consult it when deciding declaration vs expression
- Handle scoping: inner block typedefs shadow outer ones and disappear at `}`

---

## Deliverables

1. **`forge_parser` crate** — recursive descent parser producing an AST
2. **Complete C17 AST types** — every declaration, statement, expression, and GNU extension node
3. **Pratt parser** for expressions with all 15 C17 precedence levels
4. **Declaration parser** — full recursive declarator syntax including spiral-rule complexity
5. **GNU extension tolerance** — `__attribute__`, `__typeof__`, `__asm__`, `__extension__`, keyword aliases
6. **Error recovery** — synchronize on `;`, `}`, and declaration starts; collect all errors
7. **AST pretty-printer** — tree-format dump for debugging and testing
8. **Comprehensive tests** — unit, lit, stress, system header parse, real-C program

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
│   ├── type_specifiers: Vec<TypeSpecifierToken>  ← LIST, not single enum
│   ├── type_qualifiers: Vec<TypeQualifier>
│   ├── function_specifiers: Vec<FunctionSpecifier>
│   ├── alignment: Option<AlignSpec>
│   └── attributes: Vec<GnuAttribute>            ← GNU extension
│
├── Declarator  { pointers, direct }
│   └── DirectDeclarator: Ident | Paren | Array | Function
│
├── Stmt: Compound | If | While | For | Switch | Return | Goto | Label | Expr | ...
│
└── Expr (all via Pratt parser)
    ├── Literals, Ident
    ├── BinaryOp, UnaryOp, PostfixOp
    ├── FunctionCall, ArraySubscript, MemberAccess
    ├── Cast, CompoundLiteral, Conditional, Assignment
    ├── Sizeof, Alignof, GenericSelection
    └── Comma
```

### The Four Parser Ambiguities

These are the hard problems. Every one requires the typedef table.

**Ambiguity 1 — Declaration vs Expression statement:**
At block scope, `T * x;` is a pointer declaration if T is a typedef, multiplication if not. Resolved by checking `is_typedef(T)` before parsing.

**Ambiguity 2 — Cast vs Parenthesized expression vs Compound literal:**
When we see `(`:
- `(a + b)` → parenthesized expression
- `(int)x` → cast
- `(int[]){1, 2}` → compound literal

Resolution: After `(`, check if the next token starts a type-name (type keyword, qualifier, or typedef name). If yes, tentatively parse type-name → `)`. If next is `{` → compound literal. Otherwise → cast. If not a type-name → parenthesized expression.

**Ambiguity 3 — `sizeof(X)` — expression or type?**
`sizeof(T)` where T is a typedef → sizeof-type. `sizeof(x)` where x is a variable → sizeof-expression with parentheses. Resolved by checking is_typedef after `sizeof(`.

**Ambiguity 4 — Label vs Expression:**
`foo:` at statement level could be a label (if followed by a statement) or the start of a ternary expression like `foo : bar` (but this only appears mid-expression). Resolution: at statement level, if identifier is followed by `:` and we're not inside an expression, it's a label. Check `peek()` == identifier && `peek_ahead(1)` == `:` before falling through to expression-statement.

### Expression Precedence (Pratt binding powers)

```
Level  Operators                              Assoc    BP (left, right)
─────  ──────────────────────────────────────  ─────    ────────────────
15     , (comma)                               Left     (2, 3)
14     = += -= *= /= %= <<= >>= &= ^= |=     Right    (4, 3)
13     ? : (ternary)                           Right    (6, 5)
12     ||                                      Left     (8, 9)
11     &&                                      Left     (10, 11)
10     |                                       Left     (12, 13)
 9     ^                                       Left     (14, 15)
 8     &                                       Left     (16, 17)
 7     == !=                                   Left     (18, 19)
 6     < > <= >=                               Left     (20, 21)
 5     << >>                                   Left     (22, 23)
 4     + -                                     Left     (24, 25)
 3     * / %                                   Left     (26, 27)
 2     prefix: ++ -- & * + - ~ ! sizeof cast   Right    (_, 29)
 1     postfix: () [] . -> ++ --               Left     (31, _)
```

Note: `&` and `*` are both unary (prefix, bp 29) and binary (infix, bp 16/26). The Pratt parser handles this naturally — if we're in prefix position it's unary, if infix position it's binary.

---

## Acceptance Criteria

### Core
- [ ] Parse `int main() { return 0; }`
- [ ] Parse all statement types (if/else, while, do-while, for, switch/case/default, goto/label, return, break, continue, compound, empty)
- [ ] Parse all declaration forms (variables, functions, typedefs, struct, union, enum, _Static_assert, _Alignas)
- [ ] Parse complex declarators: `int (*(*fp)(int))[10]`
- [ ] Parse all expression operators with correct precedence
- [ ] Parse initializer lists with designators: `{ .x = 1, [0] = 2 }`
- [ ] Parse `_Generic`, `_Static_assert`, compound literals
- [ ] Typedef names correctly resolve all four ambiguities

### GNU Extension Tolerance
- [ ] `__attribute__((...))` skipped in all positions (specifiers, declarators, params, struct members, enumerators)
- [ ] `__extension__` consumed as no-op
- [ ] `__restrict`, `__inline__`, `__volatile__`, `__const`, `__signed__` → standard equivalents
- [ ] `__asm__(...)` skipped (balanced parens)
- [ ] `__builtin_va_list` treated as built-in typedef
- [ ] `__typeof__(expr/type)` parsed
- [ ] Preprocessed `#include <stdio.h>` output parses without errors

### Error Recovery
- [ ] Syntax error in one function doesn't prevent parsing subsequent functions
- [ ] Multiple errors collected and reported
- [ ] Missing semicolon → recovers at next statement
- [ ] No panics on any input (including empty file, garbage tokens)

---

## Claude Code Prompts

### Prompt 3.1 — AST type definitions

```
Create the forge_parser crate in the Forge workspace. Define the complete C17 AST
type hierarchy. DO NOT write the parser yet — just the types.

IMPORTANT DESIGN DECISIONS:
- Type specifiers are a Vec<TypeSpecifierToken>, NOT a single enum.
  `unsigned long long int` = four entries in the Vec.
  Resolution to concrete type happens in Phase 4 (sema).
- Use Box<> for recursive nodes. No arena allocator yet.
- Every AST node with a source location has a `span: Span` field.
- StructDef is ONE type used for both struct and union (kind field distinguishes).

Create these files:

forge_parser/src/lib.rs — crate root, pub mod declarations
forge_parser/src/ast.rs — all AST node types
forge_parser/src/ast_ops.rs — operator enums

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
AST TYPES — ast.rs
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use forge_lexer::{Span, IntSuffix, FloatSuffix, CharPrefix, StringPrefix};
use crate::ast_ops::*;

/// Root of the AST
pub struct TranslationUnit {
    pub declarations: Vec<ExternalDeclaration>,
    pub span: Span,
}

pub enum ExternalDeclaration {
    FunctionDef(FunctionDef),
    Declaration(Declaration),
}

pub struct FunctionDef {
    pub specifiers: DeclSpecifiers,
    pub declarator: Declarator,
    pub body: CompoundStmt,
    pub span: Span,
}

━━━ Declarations ━━━

pub struct Declaration {
    pub specifiers: DeclSpecifiers,
    pub init_declarators: Vec<InitDeclarator>,
    pub span: Span,
}

pub struct DeclSpecifiers {
    pub storage_class: Option<StorageClass>,
    pub type_specifiers: Vec<TypeSpecifierToken>,  // ← CRITICAL: Vec, not single
    pub type_qualifiers: Vec<TypeQualifier>,
    pub function_specifiers: Vec<FunctionSpecifier>,
    pub alignment: Option<AlignSpec>,
    pub attributes: Vec<GnuAttribute>,
    pub span: Span,
}

pub enum StorageClass { Auto, Register, Static, Extern, Typedef, ThreadLocal }

pub enum TypeSpecifierToken {
    // Primitive keywords — collected into Vec, sema resolves the combination
    Void, Char, Short, Int, Long, Float, Double,
    Signed, Unsigned, Bool, Complex,
    // Compound types
    Struct(StructDef),   // struct AND union use StructDef (kind field distinguishes)
    Union(StructDef),    // ← same type as Struct variant, kind = StructOrUnion::Union
    Enum(EnumDef),
    // Typedef reference
    TypedefName(String),
    // C11
    Atomic(Box<TypeName>),
    // GNU extensions
    TypeofExpr(Box<Expr>),
    TypeofType(Box<TypeName>),
}

pub enum TypeQualifier { Const, Volatile, Restrict, Atomic }
pub enum FunctionSpecifier { Inline, Noreturn }
pub enum AlignSpec {
    AlignAsType(TypeName),
    AlignAsExpr(Box<Expr>),
}

━━━ Declarators ━━━

pub struct InitDeclarator {
    pub declarator: Declarator,
    pub initializer: Option<Initializer>,
    pub span: Span,
}

pub struct Declarator {
    pub pointers: Vec<PointerQualifiers>,
    pub direct: DirectDeclarator,
    pub span: Span,
}

pub struct PointerQualifiers {
    pub qualifiers: Vec<TypeQualifier>,
    pub attributes: Vec<GnuAttribute>,  // __attribute__ can follow * in pointers
}

pub enum DirectDeclarator {
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
}

pub enum ArraySize {
    Unspecified,           // int arr[]
    Expr(Box<Expr>),       // int arr[10] or int arr[n] (VLA)
    VLAStar,               // int arr[*] (VLA in function prototype only)
}

pub struct ParamDecl {
    pub specifiers: DeclSpecifiers,
    pub declarator: Option<Declarator>,  // None for abstract: foo(int, int)
    pub span: Span,
}

━━━ Struct / Union / Enum ━━━

pub enum StructOrUnion { Struct, Union }

pub struct StructDef {
    pub kind: StructOrUnion,
    pub name: Option<String>,
    pub members: Option<Vec<StructMember>>,  // None = forward declaration (struct foo;)
    pub attributes: Vec<GnuAttribute>,
    pub span: Span,
}

pub enum StructMember {
    Field(StructField),
    StaticAssert(StaticAssert),  // C11: _Static_assert inside struct body
}

pub struct StructField {
    pub specifiers: DeclSpecifiers,
    pub declarators: Vec<StructFieldDeclarator>,
    pub span: Span,
}

pub struct StructFieldDeclarator {
    pub declarator: Option<Declarator>,  // None for anonymous bit-field: `int : 5;`
    pub bit_width: Option<Box<Expr>>,
    pub span: Span,
}

pub struct EnumDef {
    pub name: Option<String>,
    pub enumerators: Option<Vec<Enumerator>>,  // None = forward reference
    pub attributes: Vec<GnuAttribute>,
    pub span: Span,
}

pub struct Enumerator {
    pub name: String,
    pub value: Option<Box<Expr>>,
    pub attributes: Vec<GnuAttribute>,  // GCC allows __attribute__ on enumerators
    pub span: Span,
}

━━━ Initializers ━━━

pub enum Initializer {
    Expr(Box<Expr>),
    List {
        items: Vec<DesignatedInit>,
        span: Span,
    },
}

pub struct DesignatedInit {
    pub designators: Vec<Designator>,  // empty = no designation
    pub initializer: Box<Initializer>,
    pub span: Span,
}

pub enum Designator {
    Index(Box<Expr>),     // [expr]
    Field(String),         // .identifier
}

━━━ Statements ━━━

pub struct CompoundStmt {
    pub items: Vec<BlockItem>,
    pub span: Span,
}

pub enum BlockItem {
    Declaration(Declaration),
    Statement(Stmt),
    StaticAssert(StaticAssert),  // C11: _Static_assert at block scope
}

pub enum Stmt {
    Compound(CompoundStmt),
    Expr {
        expr: Option<Box<Expr>>,  // None = empty statement (;)
        span: Span,
    },
    If {
        condition: Box<Expr>,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
        span: Span,
    },
    While {
        condition: Box<Expr>,
        body: Box<Stmt>,
        span: Span,
    },
    DoWhile {
        body: Box<Stmt>,
        condition: Box<Expr>,
        span: Span,
    },
    For {
        init: Option<ForInit>,
        condition: Option<Box<Expr>>,
        update: Option<Box<Expr>>,
        body: Box<Stmt>,
        span: Span,
    },
    Switch {
        expr: Box<Expr>,
        body: Box<Stmt>,
        span: Span,
    },
    Case {
        value: Box<Expr>,
        body: Box<Stmt>,
        span: Span,
    },
    Default {
        body: Box<Stmt>,
        span: Span,
    },
    Return {
        value: Option<Box<Expr>>,
        span: Span,
    },
    Break { span: Span },
    Continue { span: Span },
    Goto {
        label: String,
        span: Span,
    },
    Label {
        name: String,
        stmt: Box<Stmt>,
        span: Span,
    },
}

pub enum ForInit {
    Declaration(Declaration),
    Expr(Box<Expr>),
}

pub struct StaticAssert {
    pub condition: Box<Expr>,
    pub message: Option<String>,  // C23 makes message optional
    pub span: Span,
}

━━━ Expressions ━━━

pub enum Expr {
    // Literals
    IntLiteral { value: u64, suffix: Option<IntSuffix>, span: Span },
    FloatLiteral { value: f64, suffix: Option<FloatSuffix>, span: Span },
    CharLiteral { value: u32, prefix: Option<CharPrefix>, span: Span },
    StringLiteral { value: String, prefix: Option<StringPrefix>, span: Span },
    // Names
    Ident { name: String, span: Span },
    // Binary
    BinaryOp { op: BinaryOp, left: Box<Expr>, right: Box<Expr>, span: Span },
    // Unary prefix
    UnaryOp { op: UnaryOp, operand: Box<Expr>, span: Span },
    // Unary postfix
    PostfixOp { op: PostfixOp, operand: Box<Expr>, span: Span },
    // Ternary
    Conditional { condition: Box<Expr>, then_expr: Box<Expr>, else_expr: Box<Expr>, span: Span },
    // Assignment
    Assignment { op: AssignOp, target: Box<Expr>, value: Box<Expr>, span: Span },
    // Postfix access
    FunctionCall { callee: Box<Expr>, args: Vec<Expr>, span: Span },
    MemberAccess { object: Box<Expr>, member: String, is_arrow: bool, span: Span },
    ArraySubscript { array: Box<Expr>, index: Box<Expr>, span: Span },
    // Type-related
    Cast { type_name: Box<TypeName>, expr: Box<Expr>, span: Span },
    SizeofExpr { expr: Box<Expr>, span: Span },
    SizeofType { type_name: Box<TypeName>, span: Span },
    AlignofType { type_name: Box<TypeName>, span: Span },
    CompoundLiteral { type_name: Box<TypeName>, initializer: Initializer, span: Span },
    // C11
    GenericSelection {
        controlling: Box<Expr>,
        associations: Vec<GenericAssociation>,
        span: Span,
    },
    // Comma
    Comma { exprs: Vec<Expr>, span: Span },
}

pub struct GenericAssociation {
    pub type_name: Option<TypeName>,  // None = default
    pub expr: Box<Expr>,
    pub span: Span,
}

━━━ Type Names (for sizeof, cast, compound literal, _Alignas, _Atomic) ━━━

pub struct TypeName {
    pub specifiers: DeclSpecifiers,
    pub abstract_declarator: Option<AbstractDeclarator>,
    pub span: Span,
}

pub struct AbstractDeclarator {
    pub pointers: Vec<PointerQualifiers>,
    pub direct: Option<DirectAbstractDeclarator>,
    pub span: Span,
}

pub enum DirectAbstractDeclarator {
    Parenthesized(Box<AbstractDeclarator>),
    Array {
        base: Option<Box<DirectAbstractDeclarator>>,
        size: ArraySize,
        span: Span,
    },
    Function {
        base: Option<Box<DirectAbstractDeclarator>>,
        params: Vec<ParamDecl>,
        is_variadic: bool,
        span: Span,
    },
}

━━━ GNU Extensions ━━━

pub struct GnuAttribute {
    pub name: String,
    pub args: Option<Vec<GnuAttributeArg>>,
    pub span: Span,
}

pub enum GnuAttributeArg {
    Ident(String),
    Expr(Box<Expr>),
    // Covers nested attributes: __attribute__((format(printf, 1, 2)))
    Nested { name: String, args: Vec<GnuAttributeArg> },
}

━━━ Operator enums — ast_ops.rs ━━━

pub enum BinaryOp {
    Add, Sub, Mul, Div, Mod,
    BitAnd, BitOr, BitXor,
    Shl, Shr,
    LogAnd, LogOr,
    Eq, Ne, Lt, Gt, Le, Ge,
}

pub enum UnaryOp {
    PreIncrement, PreDecrement,
    AddrOf,    // &
    Deref,     // *
    Plus,      // +
    Minus,     // -
    BitNot,    // ~
    LogNot,    // !
}

pub enum PostfixOp {
    PostIncrement,
    PostDecrement,
}

pub enum AssignOp {
    Assign,       // =
    AddAssign, SubAssign, MulAssign, DivAssign, ModAssign,
    BitAndAssign, BitOrAssign, BitXorAssign,
    ShlAssign, ShrAssign,
}

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

All types: derive Debug, Clone.
Every struct with a source location gets `pub span: Span`.

Write tests that manually construct AST nodes (verify types compile):
- FunctionDef for `int main() { return 0; }`
- Declaration for `unsigned long long x = 42;`
  → type_specifiers = vec![Unsigned, Long, Long, Int]  (verify it's a Vec)
- Expr for `a + b * c` with correct nesting
- StructDef with a bit-field member
- StructMember::StaticAssert variant (verify it exists)
- Stmt::Expr with span (verify span field exists)

Add forge_parser to workspace Cargo.toml.
Add forge_lexer and forge_diagnostics as dependencies.
```

### Prompt 3.2 — Parser infrastructure + Pratt expression parser

```
Implement parser infrastructure and the Pratt expression parser.

NOTE ON DEPENDENCY: This prompt needs parse_type_name() for cast, sizeof(type),
_Alignof(type), compound literal, and _Generic. But the full parse_type_name()
comes in Prompt 3.3. To unblock expression parsing:

→ Implement a MINIMAL parse_type_name() in this prompt that handles ONLY
  primitive type keywords: void, char, short, int, long, float, double,
  signed, unsigned, _Bool, _Complex (and combinations thereof).
  No struct/enum, no complex declarators, no typedef names.
  This is enough for cast/sizeof tests. The full version REPLACES it in 3.3.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Parser struct and helpers
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create forge_parser/src/parser.rs:

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// Stack of typedef scopes. Last = current. 
    /// An identifier is a typedef if it appears in ANY scope (walk top to bottom).
    typedefs: Vec<HashSet<String>>,
    diagnostics: Vec<Diagnostic>,
    /// Set to true when any error is emitted. Final Result uses this.
    has_errors: bool,
}

Core methods:
- peek() -> &Token
- peek_ahead(n: usize) -> &Token
- advance() -> Token
- expect(kind) -> Result<Token, ()>   // diagnostic on mismatch
- at(kind) -> bool
- eat(kind) -> Option<Token>
- at_eof() -> bool
- span_from(start: Span) -> Span      // merge start..previous token end
- save_pos() -> usize                  // for backtracking
- restore_pos(saved: usize)            // backtrack to saved position

Typedef scope:
- push_scope()
- pop_scope()
- add_typedef(name: &str)
- is_typedef(name: &str) -> bool       // check ALL scopes top to bottom
- Add __builtin_va_list to the initial scope in the constructor

Entry point:
pub fn parse(tokens: Vec<Token>) -> (TranslationUnit, Vec<Diagnostic>)
  Always returns an AST (possibly partial) + diagnostics.
  Callers check diagnostics to see if errors occurred.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Pratt expression parser
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create forge_parser/src/expr.rs with expression parsing methods on Parser.

Top-level entry points:
- parse_expr() → Expr               // full expression including comma
- parse_assignment_expr() → Expr     // no comma (used in initializers, function args)
- parse_constant_expr() → Expr       // alias for parse_conditional_expr() (used in case, enum values, bit-fields, array sizes, _Static_assert)

Core Pratt function:
  parse_pratt(min_bp: u8) → Expr

Pratt loop:
  1. lhs = parse_prefix()             // nud: literal, ident, unary, paren, etc.
  2. Loop:
     a. Look at next token's infix/postfix binding power
     b. If left_bp <= min_bp → break
     c. Consume operator token
     d. Parse right side with appropriate right_bp
     e. Build AST node, assign to lhs
  3. Return lhs

━━━ Prefix parsing (nud) ━━━

parse_prefix() dispatches on current token:

IntegerLiteral → Expr::IntLiteral { value, suffix, span }
FloatLiteral → Expr::FloatLiteral { value, suffix, span }
CharLiteral → Expr::CharLiteral { value, prefix, span }

StringLiteral → Expr::StringLiteral
  IMPORTANT — adjacent string concatenation:
  After consuming a StringLiteral, peek ahead. If next token is also StringLiteral,
  consume and concatenate. Repeat until next is not StringLiteral.
  Edge case: mixed prefixes like "hello" L"world" — for now, use the first
  string's prefix and just concatenate the text. Proper handling is a sema concern.

Identifier → Expr::Ident { name, span }

LeftParen → THREE-WAY AMBIGUITY:
  1. Save position with save_pos()
  2. Advance past `(`
  3. Check: does the next token start a type-name?
     - Type keywords: void, char, short, int, long, float, double,
       signed, unsigned, _Bool, _Complex, _Atomic, struct, union, enum
     - Type qualifiers: const, volatile, restrict
     - Known typedef name: is_typedef(token_text)
  4. If YES:
     a. Try parse_type_name() (minimal version for now)
     b. If succeed and next token is `)`:
        - Consume `)`
        - If next token is `{` → COMPOUND LITERAL:
          parse initializer list, return Expr::CompoundLiteral
        - Else → CAST:
          parse operand as parse_pratt(29) (unary precedence),
          return Expr::Cast
     c. If type-name parse fails → backtrack: restore_pos(), fall through to (5)
  5. If NO (or backtracked):
     PARENTHESIZED EXPRESSION: parse_expr(), expect `)`, return the inner expr

Prefix operators:
  PlusPlus   → Expr::UnaryOp { PreIncrement, parse_pratt(29) }
  MinusMinus → Expr::UnaryOp { PreDecrement, parse_pratt(29) }
  Ampersand  → Expr::UnaryOp { AddrOf, parse_pratt(29) }
  Star       → Expr::UnaryOp { Deref, parse_pratt(29) }
  Plus       → Expr::UnaryOp { Plus, parse_pratt(29) }
  Minus      → Expr::UnaryOp { Minus, parse_pratt(29) }
  Tilde      → Expr::UnaryOp { BitNot, parse_pratt(29) }
  Bang       → Expr::UnaryOp { LogNot, parse_pratt(29) }

sizeof:
  If next token is `(` AND token after `(` starts a type-name:
    → consume `(`, parse_type_name(), expect `)`, return Expr::SizeofType
  Else:
    → parse_pratt(29), return Expr::SizeofExpr
  Note: `sizeof(x)` where x is a typedef → SizeofType. Where x is a variable → SizeofExpr with parens. The typedef table resolves this.

_Alignof:
  Expect `(`, parse_type_name(), expect `)`, return Expr::AlignofType.
  (_Alignof only applies to types, never expressions, in C11.)

_Generic:
  _Generic `(` assignment-expr `,` generic-assoc-list `)`
  Where generic-assoc is:
    type-name `:` assignment-expr
    | `default` `:` assignment-expr
  Comma-separated. At most one `default`.
  NOTE: type-name here uses the minimal parse_type_name().
  Build Expr::GenericSelection.

━━━ Infix/postfix parsing (led) ━━━

Binary operators — all use the binding powers from the table:
  + - * / % << >> < > <= >= == != & ^ | && ||
  Each: consume op, rhs = parse_pratt(right_bp), build Expr::BinaryOp

Ternary ? :
  After `?`: parse full expression (including comma — C allows it in middle),
  expect `:`, parse_pratt(right_bp=5) for the else branch.
  Right-associative: `a ? b : c ? d : e` = Cond(a, b, Cond(c, d, e))

Assignment operators = += -= *= /= %= <<= >>= &= ^= |=
  Right-associative (right_bp=3): rhs = parse_pratt(3)
  Build Expr::Assignment

Comma operator:
  Left-associative (right_bp=3). Build Expr::Comma.
  Collect multiple: a, b, c → Comma([a, b, c])

Postfix operators:
  LeftParen → FUNCTION CALL:
    Parse argument list (comma-separated assignment-exprs), expect `)`
    Build Expr::FunctionCall

  LeftBracket → ARRAY SUBSCRIPT:
    Parse expression, expect `]`
    Build Expr::ArraySubscript

  Dot → MEMBER ACCESS:
    Expect identifier, build Expr::MemberAccess { is_arrow: false }

  Arrow → MEMBER ACCESS:
    Expect identifier, build Expr::MemberAccess { is_arrow: true }

  PlusPlus → Expr::PostfixOp { PostIncrement }
  MinusMinus → Expr::PostfixOp { PostDecrement }

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Minimal parse_type_name (temporary)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

This is a TEMPORARY minimal version. Prompt 3.3 replaces it with the full version.

parse_type_name_minimal() → Option<TypeName>:
  Collect type specifier keywords (void, char, short, int, long, float, double,
  signed, unsigned, _Bool, _Complex) and type qualifiers (const, volatile, restrict).
  Then optionally parse pointer prefix(es): * with optional qualifiers.
  Return TypeName with the collected specifiers and optional abstract declarator.

  This handles: (int), (const int), (unsigned long long), (int *), (const char **)
  Does NOT handle: (struct foo), (enum bar), (int [10]), (int (*)(int))
  Those come in 3.3.

Mark with a comment: // TODO(3.3): replace with full parse_type_name

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Helper: create a function that takes a C expression string, wraps it in
a minimal context to lex+preprocess, then calls parse_expr(), returns Expr.

Precedence tests:
- `1 + 2 * 3` → Add(1, Mul(2, 3))
- `1 * 2 + 3` → Add(Mul(1, 2), 3)
- `a + b + c` → Add(Add(a, b), c)  (left assoc)
- `a = b = c` → Assign(a, Assign(b, c))  (right assoc)
- `a - b - c` → Sub(Sub(a, b), c)  (left assoc)

Ternary:
- `a ? b : c` → Conditional(a, b, c)
- `a ? b : c ? d : e` → Conditional(a, b, Conditional(c, d, e))

Unary:
- `-x` → UnaryOp(Minus, Ident(x))
- `!a && b` → LogAnd(LogNot(a), b)
- `*p++` → Deref(PostIncrement(p))
- `-!x` → Minus(LogNot(x))
- `++*p` → PreIncrement(Deref(p))

Postfix:
- `a[0]` → ArraySubscript(a, 0)
- `a.b` → MemberAccess(a, "b", arrow=false)
- `a->b` → MemberAccess(a, "b", arrow=true)
- `a[0].b->c++` → correct nesting (left to right)
- `f(a, b, c)` → FunctionCall(f, [a, b, c])
- `f()` → FunctionCall(f, [])

Cast (using minimal type-name):
- `(int)x` → Cast(Int, x)
- `(unsigned long)x` → Cast([Unsigned, Long], x)
- `(int *)(void *)p` → Cast(int*, Cast(void*, p))

Sizeof:
- `sizeof(int)` → SizeofType(Int)
- `sizeof x` → SizeofExpr(Ident(x))
- `sizeof(x)` → SizeofExpr(Ident(x))  (x is NOT a typedef, so it's expr)

Compound literal (minimal):
- `(int){42}` → CompoundLiteral(Int, [42])

String concatenation:
- `"hello" " " "world"` → StringLiteral("hello world")

Comma:
- `a, b, c` → Comma([a, b, c])

Complex:
- `*p++ = f(a + b, c)` → correct tree
- `a || b && c` → LogOr(a, LogAnd(b, c))
- `a & b == c` → BitAnd(a, Eq(b, c))  (== binds tighter than &)

_Generic (using minimal type-name):
- `_Generic(x, int: 1, float: 2, default: 0)`
```

### Prompt 3.3 — Declaration specifiers + declarators + typedef tracking

```
Implement declaration specifier parsing, simple declarators, typedef tracking,
and the FULL parse_type_name() (replacing the minimal version from 3.2).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Declaration specifiers
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_declaration_specifiers() → DeclSpecifiers

Loop collecting specifiers. On each iteration, check the current token:

Storage class: auto, register, static, extern, typedef, _Thread_local
  → Only ONE storage class per declaration. Second one → error diagnostic.
  → Store in storage_class field.

Type specifier keywords: void, char, short, int, long, float, double,
  signed, unsigned, _Bool, _Complex
  → Push into type_specifiers Vec.
  → Multiple are legal: `unsigned long long int` = 4 entries.
  → Invalid combos (float double) detected by sema, NOT here.

Type qualifiers: const, volatile, restrict
  → Push into type_qualifiers Vec.
  → Duplicates allowed (redundant but legal in C).

_Atomic:
  → _Atomic followed by `(` → parse_type_name in parens → TypeSpecifierToken::Atomic
  → _Atomic NOT followed by `(` → TypeQualifier::Atomic

Function specifiers: inline, _Noreturn
  → Push into function_specifiers Vec.

_Alignas:
  → _Alignas `(` → try type-name first; if fails, parse expression → AlignSpec

struct / union / enum:
  → Call parse_struct_or_union_specifier() / parse_enum_specifier()
    (placeholder stub for now — implemented in 3.4)
  → Push result into type_specifiers Vec.

Typedef name resolution (CRITICAL):
  If current token is an Identifier AND is_typedef(name) returns true:
    → ONLY treat as typedef if we haven't seen another type specifier yet.
    → Edge case: `typedef int T; { T T; }` — second `T T;` means
      "variable named T of type T". Once we've already collected a type 
      specifier (the first T as TypedefName), the second T is a declarator name.
    → Implementation: track a `bool seen_type_specifier` in the loop.
      If seen_type_specifier is true, STOP — don't treat the identifier as typedef.
    → Push TypeSpecifierToken::TypedefName(name) into type_specifiers.

__attribute__((...)) → call skip_gnu_attributes() (placeholder for 3.6)

GNU keyword equivalents (handle these NOW, not in 3.6, because they appear in
declaration specifiers of system headers even before 3.6):
  __const, __const__       → TypeQualifier::Const
  __volatile, __volatile__ → TypeQualifier::Volatile
  __restrict, __restrict__ → TypeQualifier::Restrict
  __inline, __inline__     → FunctionSpecifier::Inline
  __signed, __signed__     → TypeSpecifierToken::Signed
  __extension__            → consume and continue (no-op)

Loop termination: stop when token is NOT a specifier keyword, qualifier, 
typedef name, or GNU keyword.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Declarators
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_declarator() → Declarator

Structure: pointer-prefix* direct-declarator suffix*

Pointer prefix:
  While current token is `*`:
    Consume `*`
    Collect optional qualifiers (const, volatile, restrict, _Atomic)
    Also handle __attribute__ after * (e.g., `* __attribute__((aligned)) p`)
    Push PointerQualifiers { qualifiers, attributes }

Direct declarator (parse_direct_declarator):
  Base:
    Identifier → DirectDeclarator::Identifier(name, span)
    LeftParen → DirectDeclarator::Parenthesized(Box<parse_declarator()>)
      Consume `)`. This handles `(*fp)` grouping.

  Suffixes (loop until no more [ or ( ):
    LeftBracket → Array suffix:
      `[]` → ArraySize::Unspecified
      `[*]` → ArraySize::VLAStar  (only in function prototypes)
      `[static expr]` → is_static=true, ArraySize::Expr
      `[const expr]` / `[restrict expr]` → qualifiers + ArraySize::Expr
      `[expr]` → ArraySize::Expr
      Expect `]`
      Wrap current in DirectDeclarator::Array { base: current, ... }

    LeftParen → Function suffix:
      Parse parameter list (Section 3)
      Wrap current in DirectDeclarator::Function { base: current, params, is_variadic }

parse_abstract_declarator() → AbstractDeclarator
  Same structure but no identifier. Used in type-names (sizeof, cast, etc.)
  Direct abstract declarator: can start with `(` (paren group) or `[` (array)
  or `(` (function params), with no identifier at the base.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Parameter list
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_parameter_list() → (Vec<ParamDecl>, bool)

After `(`:
- `(void)` → zero params, not variadic
  (only when `void` is the single token before `)`)
- `()` → zero params, not variadic (C treats as unspecified params)
- Otherwise:
  Loop parsing parameter declarations:
    1. Parse declaration specifiers
    2. Optionally parse declarator or abstract-declarator
       - If the next non-pointer token is Identifier → parse_declarator()
       - Otherwise → parse_abstract_declarator() (or nothing)
    3. If next token is `,`:
       - If the token after `,` is `...` → consume `,` and `...`, set is_variadic=true, break
       - Otherwise → consume `,`, continue loop
    4. If next token is `)` → break

Expect `)` at the end.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Full parse_type_name (replaces minimal version)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_type_name() → TypeName

REPLACE the minimal version from 3.2 with:
  1. Parse declaration specifiers (now handles struct/union/enum too, once 3.4 stubs are in)
  2. Optionally parse abstract declarator

This is used by:
  - Cast expressions: (type-name) expr
  - Compound literals: (type-name) { ... }
  - sizeof(type-name)
  - _Alignof(type-name)
  - _Alignas(type-name)
  - _Atomic(type-name)
  - _Generic associations

After replacing, all expression tests from 3.2 should still pass, and now
casts with complex types work: (struct Point *)p, (int (*)(int))fp, etc.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Declaration and init-declarator list
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_declaration() → Declaration

1. Parse declaration specifiers
2. If followed by `;` → empty declaration (e.g., `struct foo;`, `enum bar;`)
3. Otherwise parse init-declarator-list:
   - parse_declarator()
   - If `=` follows → parse_initializer()
     (for now, only Initializer::Expr — brace-init comes in 3.4)
   - If `,` follows → continue to next init-declarator
   - Expect `;` at end

4. TYPEDEF TRACKING (critical):
   If specifiers.storage_class == Some(Typedef):
     For each declarator, extract the declared name:
       Walk the DirectDeclarator tree to find the Identifier node.
       Helper: fn declarator_name(d: &Declarator) -> Option<&str>
     Call add_typedef(name) for each.

   Examples:
     `typedef unsigned long size_t;` → add "size_t"
     `typedef int (*handler_t)(int);` → add "handler_t"
     `typedef int T, *PT;` → add "T" AND "PT"

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 6 — Declaration vs expression ambiguity
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

is_start_of_declaration() → bool

Peek at current token:
  Type keywords → true
  Storage class keywords → true
  Type qualifiers (const, volatile, restrict) at start → true
  _Alignas → true
  _Static_assert → true
  struct, union, enum → true
  Known typedef name (is_typedef) → true
  __attribute__ → true
  __extension__ → true (often precedes typedef/declaration in headers)
  GNU type keywords (__signed__, __const, etc.) → true
  Anything else → false

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 7 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Helper: parse_decl(source) → parse a declaration string, return Declaration.

- `int x;` → specifiers=[Int], declarator "x", no initializer
- `unsigned long long x;` → specifiers=[Unsigned, Long, Long], declarator "x"
- `long unsigned int long x;` → specifiers=[Long, Unsigned, Int, Long] (order preserved)
- `const int *p;` → qualifiers=[Const], specifiers=[Int], 1 pointer level
- `int *const p;` → declarator has pointer with Const qualifier
- `int **const *volatile p;` → three pointer levels with correct qualifiers
- `int x = 5;` → initializer IntLiteral(5)
- `int x = 1 + 2;` → initializer BinaryOp(Add, 1, 2)
- `int x, y, *z;` → three init-declarators
- `typedef int MyInt;` → "MyInt" in typedef set
- `MyInt x;` after typedef → type_specifiers=[TypedefName("MyInt")]
- `typedef int T; T * x;` → x is pointer-to-T declaration (NOT multiplication)
- `int x; x * y;` → second is expression statement (multiplication)
- `typedef int T; { T T; }` → inner: type T, name T (edge case)
- `int (*fp);` → parenthesized declarator
- `int f(int a, char *b);` → function declarator with 2 params
- `int f(void);` → function with no params
- `int f(int, ...);` → variadic, abstract first param
- `int arr[10];` → array with size=Expr(10)
- `int arr[];` → array with size=Unspecified
- `int (*fp)(int, int);` → pointer to function
- `int (*arr[10])(void);` → array of 10 function pointers

Re-run all Prompt 3.2 expression tests — they must still pass.
```

### Prompt 3.4 — Struct/union/enum definitions + initializer lists

```
Implement struct/union/enum definitions and brace-enclosed initializer lists.
Replace the placeholder stubs from 3.3.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Struct and union definitions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_struct_or_union_specifier(kind: StructOrUnion) → TypeSpecifierToken

After `struct` or `union` keyword:
1. Optional __attribute__((...)) → skip for now (placeholder)
2. Optional name (identifier)
3. If followed by `{`:
   parse_struct_member_list():
   Loop until `}`:
     - If _Static_assert → parse_static_assert(), push StructMember::StaticAssert
     - Otherwise:
       a. Parse declaration specifiers
       b. Parse struct-declarator-list (comma-separated):
          - Optional declarator (can be None for anonymous bit-field)
          - Optional `: constant-expr` for bit-width
          - `int x;` → declarator=Some("x"), bit_width=None
          - `int x : 3;` → declarator=Some("x"), bit_width=Some(3)
          - `int : 5;` → declarator=None, bit_width=Some(5)
       c. Expect `;`
       d. Push StructMember::Field
4. If no `{`:
   Forward declaration: `struct foo` (name only, members=None)
   Must have a name — `struct;` is an error.

5. Optional trailing __attribute__ → skip

Build StructDef { kind, name, members, attributes, span }

Anonymous struct/union members (C11):
  `struct S { union { int x; float f; }; int y; };`
  The inner union has no declarator — this is already handled because
  the struct-declarator-list can have declarator=None.
  But there's a subtlety: the member declaration has specifiers (union {...})
  and NO declarator at all (not even an anonymous bit-field). Handle this:
  if after specifiers we see `;` directly, create a StructField with an
  empty declarators vec.

Flexible array member:
  `struct S { int n; int data[]; };`
  The last member can have ArraySize::Unspecified. Parser allows it;
  sema validates it's actually the last member.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Enum definitions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_enum_specifier() → TypeSpecifierToken

After `enum`:
1. Optional __attribute__ → skip
2. Optional name
3. If followed by `{`:
   Parse enumerator list (comma-separated):
     name [= constant-expr] [__attribute__(...)]
   Trailing comma before `}` is ALLOWED (C99+):
     `enum { A, B, C, }` — valid
   Empty enum `enum {}` — invalid, emit error
4. If no `{`:
   Forward reference: `enum color` (name only)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Initializer lists
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Extend parse_initializer() (currently only handles Expr):

parse_initializer() → Initializer:
  If `{` → parse_initializer_list()
  Else → Initializer::Expr(parse_assignment_expr())

parse_initializer_list() → Initializer::List:
  After `{`:
  Loop:
    1. Parse optional designator list:
       `.field` → Designator::Field
       `[expr]` → Designator::Index
       Can be chained: `.pos[0].x` = [Field("pos"), Index(0), Field("x")]
       Designators followed by `=`
    2. Parse initializer (recursive — can be nested `{...}`)
    3. Push DesignatedInit { designators, initializer }
    4. If `,` → consume and continue (but if next is `}` → trailing comma, break)
    5. If `}` → break
  Expect `}`

  Empty initializer list `{}` — technically not valid in C17 (valid in C23).
  Accept it with no error (many compilers allow it as extension).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Struct/Union:
- `struct Point { int x; int y; };` → 2 members
- `struct { int x; } anon;` → anonymous struct
- `struct Flags { unsigned a : 1; unsigned b : 3; };` → bit-fields
- `struct { int : 4; int x : 4; };` → anonymous bit-field + named
- `struct Node { int val; struct Node *next; };` → self-referential (forward ref)
- `struct Outer { struct Inner { int x; } inner; int y; };` → nested struct def
- `union Val { int i; float f; double d; };` → union
- `struct S { union { int x; float f; }; int y; };` → anonymous union member (C11)
- `struct S { int n; int data[]; };` → flexible array member
- `struct S { _Static_assert(sizeof(int) == 4, "oops"); int x; };` → static assert in struct
- `struct S;` → forward declaration

Enum:
- `enum Color { RED, GREEN, BLUE };` → 3 enumerators, no values
- `enum { A = 0, B = 5, C };` → with explicit values
- `enum E { X, Y, Z, };` → trailing comma is valid
- `enum E;` → forward reference

Initializer lists:
- `int a[] = {1, 2, 3};` → List with 3 items
- `int a[2][2] = { {1, 2}, {3, 4} };` → nested lists
- `struct Point p = { .x = 1, .y = 2 };` → field designators
- `int a[10] = { [5] = 50, [9] = 90 };` → index designators
- `struct { struct Point pos; } s = { .pos = { .x = 1, .y = 2 } };` → nested designated
- `struct Point p = { .x = 1, .y = 2, };` → trailing comma
- `int a[] = {};` → empty initializer list (extension)

Complex declarations (full pipeline):
- `int (*(*fp)(int))[10];` → pointer to function returning pointer to array of 10 ints
- `void (*signal(int sig, void (*func)(int)))(int);` → signal function signature
- `int (*fps[10])(void);` → array of 10 function pointers

Re-run ALL previous tests (3.2 + 3.3). Must still pass.
```

### Prompt 3.5 — Statements, top-level parsing, error recovery

```
Implement statement parsing, translation-unit parsing, and error recovery.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — Statement parsing
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_statement() → Stmt

Dispatch on current token:

  `{` → parse_compound_statement()
  `if` → parse_if_statement()
  `while` → parse_while_statement()
  `do` → parse_do_while_statement()
  `for` → parse_for_statement()
  `switch` → parse_switch_statement()
  `case` → parse_case_statement()
  `default` → parse_default_statement()
  `return` → parse_return_statement()
  `break` → Stmt::Break { span }, expect `;`
  `continue` → Stmt::Continue { span }, expect `;`
  `goto` → Stmt::Goto { label, span }, expect `;`
  `;` → Stmt::Expr { expr: None, span }  (empty statement)

  Identifier:
    LABEL AMBIGUITY: check if next token is `:`
    If peek() is Identifier AND peek_ahead(1) is Colon:
      → consume identifier, consume `:`, parse_statement() for the body
      → Stmt::Label { name, stmt, span }
    Else:
      → fall through to expression-statement

  Otherwise → expression-statement: parse_expr(), expect `;`
    → Stmt::Expr { expr: Some(expr), span }

Compound statement:
  `{` block-item* `}`
  SCOPE: push_scope() after `{`, pop_scope() before `}`
  parse_block_item():
    _Static_assert → parse and emit as BlockItem::StaticAssert
    is_start_of_declaration() → BlockItem::Declaration(parse_declaration())
    else → BlockItem::Statement(parse_statement())

If statement:
  `if` `(` expr `)` stmt [`else` stmt]
  Dangling else: naturally handled by recursive descent — `else` binds to
  the innermost `if`.

While:
  `while` `(` expr `)` stmt

Do-while:
  `do` stmt `while` `(` expr `)` `;`

For:
  `for` `(`
  SCOPE: push_scope() before parsing init (declarations in init are scoped to the for)

  Init:
    `;` → no init
    is_start_of_declaration() → ForInit::Declaration(parse_declaration())
      NOTE: parse_declaration() already consumes the `;`, so don't expect another
    else → ForInit::Expr(parse_expr()), expect `;`

  Condition:
    `;` → no condition
    else → parse_expr(), expect `;`

  Update:
    `)` → no update
    else → parse_expr()  (NO semicolon here — `)` terminates)

  Expect `)`
  Body: parse_statement()
  pop_scope() after body

  IMPORTANT: Do NOT push a second scope inside parse_declaration() for the init.
  The for-scope covers both the init-declaration and the body.

Switch:
  `switch` `(` expr `)` stmt

Case:
  `case` constant-expr `:` stmt

Default:
  `default` `:` stmt

Return:
  `return` [expr] `;`
  If next token is `;` → no return value
  Else → parse_expr(), expect `;`

_Static_assert (at statement level):
  `_Static_assert` `(` constant-expr [`,` string-literal] `)` `;`

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Top-level parsing
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

parse_translation_unit() → TranslationUnit

Loop until EOF:
  Skip any stray `;` at top level (with warning — but don't error)
  parse_external_declaration()

parse_external_declaration() → ExternalDeclaration:

  _Static_assert → parse and wrap as Declaration (or add ExternalDeclaration::StaticAssert variant)

  1. Parse declaration specifiers
  2. If `;` → empty declaration (e.g., `struct foo;` at file scope)
  3. Parse first declarator
  4. FUNCTION DEFINITION vs DECLARATION:
     - Look at the declarator: does it have a Function suffix?
     - Look at the next token: is it `{`?
     - If BOTH → FunctionDef:
       Parse compound statement as body.
       (Specifiers + declarator define the function signature)
     - Otherwise → Declaration:
       Parse rest of init-declarator-list (comma, more declarators, `=` init)
       Expect `;`

  5. After each declaration/function-def, check for typedef and update the set
     (same logic as parse_declaration in 3.3).

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — Error recovery
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

When expect() fails or an unexpected token is encountered:
1. Emit diagnostic with span and descriptive message
2. Call synchronize()

synchronize():
  Skip tokens until one of:
    `;` → consume it, return (end of statement)
    `}` → do NOT consume (might close an enclosing block)
    `{` → do NOT consume (might start a function body)
    Token at start of line that looks like a new declaration
      (type keyword, storage class, struct/union/enum) → do NOT consume
    EOF → return

  After synchronizing, the parser resumes from the next clean point.

Error tracking:
  Every diagnostic emitted sets has_errors = true.
  The returned TranslationUnit may be incomplete (missing subtrees),
  but it should not contain garbage — use Option or omit broken nodes.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Functions:
- `int main() { return 0; }` → FunctionDef
- `int add(int a, int b) { return a + b; }` → params + body
- `void f(void) {}` → void return, void params, empty body

Statements:
- `if (x) y();` → If without else
- `if (x) y(); else z();` → If with else
- `if (a) if (b) c(); else d();` → dangling else binds to inner if
- `while (1) { break; }` → While + Break
- `do { x++; } while (x < 10);` → DoWhile
- `for (int i = 0; i < 10; i++) sum += i;` → For with decl init
- `for (i = 0; i < 10; i++) {}` → For with expr init
- `for (;;) break;` → infinite for, all parts empty
- `switch (x) { case 1: a(); break; case 2: b(); break; default: c(); }` → Switch
- `goto end; end: return;` → Goto + Label
- `;` → empty statement
- `{ int x = 1; int y = 2; }` → compound with 2 declarations

Scoping:
- `typedef int T; { T x; { typedef float T; T y; } T z; }`
  → outer T=int, inner T=float, after inner block T=int again

Top-level:
- Multiple functions → Vec of FunctionDefs
- Mix of declarations and functions
- `extern int x;` at file scope → Declaration

Error recovery:
- `int x = ;` → error at `;`, recovers, continues
- `int f() { int x = 5 int y = 6; }` → missing `;`, error, recovers
- `int f() { @@@ } int g() { return 1; }` → garbage in f, but g parses ok
- Multiple errors → all collected in diagnostics
- No panics on any of these

Re-run ALL previous tests (3.2 + 3.3 + 3.4). Must still pass.
```

### Prompt 3.6 — GNU extension tolerance + AST printer + driver integration

```
Add full GNU extension tolerance, implement the AST pretty-printer, and wire
the parser into the Forge driver.

MOTIVATION: System headers use GNU extensions everywhere. Without handling them,
`forge check` or `forge parse` on any file that includes a system header fails.
This is the parser's "boss fight" — the equivalent of predefined macros in Phase 2.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 1 — __attribute__((...)) handling
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

skip_gnu_attributes():
  When you see identifier `__attribute__` or `__attribute`:
  1. Expect `(`, Expect `(` (double parens: `__attribute__((...))`)
  2. Count balanced parens. Consume EVERYTHING until `))`
     (the inner contents can be arbitrarily complex — identifiers, strings,
      numbers, commas, nested parens, etc.)
  3. Return (optionally store in GnuAttribute list, but OK to discard for Phase 3)

Call sites — skip_gnu_attributes() must be called at ALL of these positions:
  ☐ In parse_declaration_specifiers() loop, when token is __attribute__
  ☐ After parse_declarator() completes (attributes on the declarator)
  ☐ After `)` closing a function parameter list
  ☐ Before `{` of struct/union body (attributes on struct)
  ☐ After `}` of struct/union body
  ☐ Before `{` of enum body
  ☐ After each enumerator (before `,` or `}`)
  ☐ On struct members, after the declarator
  ☐ On function parameters, after the declarator

If you miss ANY of these positions, a system header will trigger a parse error.
When in doubt, add an extra check: after consuming any declarator or specifier
sequence, peek for __attribute__ and skip it.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 2 — Other GNU extensions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

__extension__:
  Wherever it appears (before a declaration, before an expression, before a
  statement), consume it and continue parsing. It's a no-op semantically.
  It can appear in:
  - Top level: `__extension__ typedef ...`
  - Expression: `__extension__ (expr)`
  - In parse_prefix() of the expression parser, handle it as: consume, then
    return parse_pratt(current_bp)

__typeof__(expr) and __typeof(expr) and typeof(expr):
  Already handled as TypeSpecifierToken::TypeofExpr/TypeofType in 3.3.
  Verify it works.

__builtin_va_list:
  Should already be in the initial typedef set (from 3.2 Parser constructor).
  Verify it works: `__builtin_va_list ap;` should parse as a declaration.

__asm__ / asm / __asm:
  Can appear after a function declarator: `int foo(void) __asm__("_foo");`
  Can appear after a variable declarator: `int x __asm__("my_x");`
  Implementation: after parse_declarator(), check for __asm__/__asm/asm.
  If found: consume balanced parens and discard. Continue parsing.

  Also handle asm at statement level (GCC inline asm):
  `__asm__("nop");` or `asm volatile("..." : : : "memory");`
  For now: detect `asm`/`__asm__`/`__asm` at statement level, consume all
  balanced parens and the closing `;`, emit Stmt::Expr(None) or a new
  Stmt::GnuAsm variant (up to you — both work, sema doesn't need it yet).

__builtin_offsetof(type, member):
  Appears as an expression. When parse_prefix() sees `__builtin_offsetof`:
  Consume `(`, parse type-name, consume `,`, parse member expression
  (can be `member.field` or `member[idx]`), consume `)`.
  Return a synthetic Expr (e.g., Expr::IntLiteral(0) as placeholder, or
  add an Expr::BuiltinOffsetof variant if you prefer).

__builtin_types_compatible_p(type1, type2):
  Similar to offsetof: consume `(`, parse two type-names separated by `,`,
  consume `)`. Return a placeholder expression.

Other __builtin_* functions:
  These typically look like function calls: `__builtin_expect(expr, val)`.
  The expression parser already handles function calls, so these work
  automatically — `__builtin_expect` parses as Ident, then `(...)` parses as
  FunctionCall. No special handling needed.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 3 — AST pretty-printer
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Create forge_parser/src/printer.rs:
  pub fn print_ast(tu: &TranslationUnit) -> String

Output format — indented tree with 2-space indent:

TranslationUnit
  FunctionDef "main" → [Int]
    Params: (void)
    Body:
      CompoundStmt
        Return
          IntLiteral 0

  Declaration [Unsigned, Long, Long] "x"
    Initializer: IntLiteral 42

  StructDef "Point"
    Field [Int] "x"
    Field [Int] "y"

Show:
- Type specifiers as list: [Unsigned, Long, Long]
- Pointer levels: *const *
- Array sizes: [10], [], [*]
- Function params: (int a, char *b, ...)
- Expression trees with operator names: BinaryOp(Add, IntLiteral(1), IntLiteral(2))
- Designated initializers: .field = ..., [0] = ...

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 4 — Driver integration
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Update forge_driver:
1. After preprocessing, feed token stream to parser
2. `forge check file.c` → lex + preprocess + parse, report all diagnostics
3. Add `forge parse file.c` subcommand → dumps AST tree (from printer)
4. `forge -E` unchanged (preprocess only)

Diagnostics from all phases (lexer, preprocessor, parser) are combined and
reported together.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
SECTION 5 — Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

A. GNU __attribute__:
   - `int x __attribute__((aligned(16)));` → parses
   - `void f(void) __attribute__((noreturn));` → parses
   - `struct __attribute__((packed)) S { int x; };` → parses
   - `__attribute__((visibility("default"))) int x;` → parses
   - `void f(int x __attribute__((unused)));` → parses (param attribute)
   - `enum E { A __attribute__((deprecated)), B };` → parses (enumerator attribute)
   - `int * __attribute__((aligned(8))) p;` → attribute on pointer level

B. GNU keywords:
   - `__inline__ int f(void) { return 0; }` → FunctionSpecifier::Inline
   - `int * __restrict p;` → TypeQualifier::Restrict
   - `__extension__ typedef int __int128_t;` → typedef parsed

C. __typeof__:
   - `__typeof__(1 + 2) x;` → TypeofExpr
   - `__typeof__(int *) p;` → TypeofType

D. __asm__:
   - `extern int foo __asm__("_foo");` → declaration parses
   - `__asm__("nop");` as a statement → parses (emits Stmt or skips)

E. __builtin_*:
   - `__builtin_va_list ap;` → declaration (va_list is typedef)
   - `__builtin_offsetof(struct S, field)` → parses as expression
   - `__builtin_expect(x, 0)` → parses as function call (automatic)

F. SYSTEM HEADER SMOKE TEST (THE BIG TEST):
   Create a test file:
   ```c
   #include <stdio.h>
   #include <stdlib.h>
   #include <string.h>
   #include <stdint.h>
   int main(void) { return 0; }
   ```
   Pipeline: lex → preprocess → parse.
   Assert: ZERO parse errors.

   If this fails, the error messages will tell you which construct is unhandled.
   Common fixes needed:
   - __attribute__ in a position you missed → add skip_gnu_attributes() call
   - __asm__ after a declarator you didn't handle → add __asm__ skip
   - Unknown __builtin_* → usually parses as function call automatically
   - Inline asm at unexpected position → add statement-level asm handling

   ITERATE until it passes. This is the most important single test.

G. AST printer:
   - Parse `int main() { return 0; }`, verify printed tree looks correct.
   - Parse struct, enum, function with params, verify output.

Run cargo test --all, cargo clippy, cargo fmt.
```

### Prompt 3.7 — Full validation

```
Run comprehensive validation of forge_parser before moving to Phase 4.
Same pattern as Phase 2.8 — code audit, completeness, stress, real-world, performance.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 1 — Code Audit
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

A. unwrap()/expect() audit:
   List ALL in forge_parser. Replace with error handling or justify with comment.
   The parser must NEVER panic on any input.

B. TODO/FIXME audit:
   Resolve or document in KNOWN_ISSUES.md.

C. Clippy pedantic:
   cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic
   Fix or suppress-with-justification.

D. Dead code check:
   Unused pub functions? AST variants with no test coverage?

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 2 — Completeness Matrix
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

For EVERY feature, verify test exists AND passes. Write any missing tests.

DECLARATIONS:
| Feature                                  | Test? | Pass? |
|------------------------------------------|-------|-------|
| int x;                                   |       |       |
| int x = 5;                               |       |       |
| int x, y, *z;                            |       |       |
| unsigned long long x;                    |       |       |
| long unsigned int long x; (any order)    |       |       |
| const int *p;                            |       |       |
| int *const p;                            |       |       |
| int *const *volatile p;                  |       |       |
| typedef int MyInt;                       |       |       |
| MyInt x; (typedef resolves)              |       |       |
| typedef int T; T * x; (not multiply)    |       |       |
| typedef int T; { T T; } (edge case)     |       |       |
| static int x;                            |       |       |
| extern int x;                            |       |       |
| _Thread_local int x;                     |       |       |
| int arr[10];                             |       |       |
| int arr[];                               |       |       |
| int arr[n]; (VLA)                        |       |       |
| int f(int a, char *b);                   |       |       |
| int f(void);                             |       |       |
| int f(int, ...);                         |       |       |
| int (*fp)(int);                          |       |       |
| int (*(*fp)(int))[10];                   |       |       |
| int (*fps[10])(void);                    |       |       |
| struct Point { int x; int y; };          |       |       |
| struct Point p; (use after def)          |       |       |
| struct { int x; } anon;                  |       |       |
| union { int i; float f; };              |       |       |
| struct with anonymous union member (C11) |       |       |
| struct with flexible array member        |       |       |
| struct with _Static_assert member        |       |       |
| struct with bit-fields                   |       |       |
| Anonymous bit-field (int : 5;)           |       |       |
| enum Color { RED, GREEN, BLUE };         |       |       |
| enum with explicit values                |       |       |
| enum with trailing comma                 |       |       |
| _Static_assert(...)                      |       |       |
| _Alignas(16) int x;                     |       |       |
| _Alignas(int) char c;                   |       |       |
| _Atomic int x;                          |       |       |
| _Atomic(int) x;                         |       |       |

EXPRESSIONS:
| Feature                                  | Test? | Pass? |
|------------------------------------------|-------|-------|
| Arithmetic precedence correct            |       |       |
| All 17 binary operators                  |       |       |
| All 8 unary operators                    |       |       |
| All 11 assignment operators              |       |       |
| Ternary ? : (right-associative)          |       |       |
| Assignment (right-associative)           |       |       |
| Left-associative (a - b - c)             |       |       |
| Function call with 0, 1, N args          |       |       |
| Nested function calls f(g(x))            |       |       |
| Array subscript a[i]                     |       |       |
| Chained postfix a[0].b->c               |       |       |
| Post-increment/decrement                |       |       |
| Pre-increment/decrement                 |       |       |
| Cast (int)x                              |       |       |
| Cast (int *)(void *)p                   |       |       |
| sizeof(expr)                             |       |       |
| sizeof(type)                             |       |       |
| sizeof a (no parens)                     |       |       |
| _Alignof(type)                           |       |       |
| Compound literal (int){42}              |       |       |
| Compound literal (int[]){1,2,3}         |       |       |
| _Generic selection                       |       |       |
| String concatenation "a" "b"            |       |       |
| Comma expression a, b, c               |       |       |
| Complex: *p++ = f(a + b, c)            |       |       |

STATEMENTS:
| Feature                                  | Test? | Pass? |
|------------------------------------------|-------|-------|
| Compound statement                       |       |       |
| if without else                          |       |       |
| if with else                             |       |       |
| Dangling else (nested if)                |       |       |
| while                                    |       |       |
| do-while                                 |       |       |
| for with expr init                       |       |       |
| for with decl init                       |       |       |
| for(;;) (infinite)                       |       |       |
| switch / case / default                  |       |       |
| Multiple case labels                     |       |       |
| goto / label                             |       |       |
| return with value                        |       |       |
| return without value                     |       |       |
| break / continue                         |       |       |
| Empty statement ;                        |       |       |
| _Static_assert at block scope            |       |       |
| Nested blocks with typedef scoping       |       |       |

GNU EXTENSIONS:
| Feature                                  | Test? | Pass? |
|------------------------------------------|-------|-------|
| __attribute__((...)) on declaration      |       |       |
| __attribute__ on declarator              |       |       |
| __attribute__ on function params         |       |       |
| __attribute__ on struct/enum             |       |       |
| __attribute__ on enumerators             |       |       |
| __extension__                             |       |       |
| __restrict / __inline__ / __volatile__   |       |       |
| __signed__                                |       |       |
| __const / __const__                       |       |       |
| __typeof__(expr)                          |       |       |
| __typeof__(type)                          |       |       |
| __asm__(...) on declaration              |       |       |
| __asm__ as statement                      |       |       |
| __builtin_va_list                         |       |       |
| __builtin_offsetof                        |       |       |
| __builtin_* as function calls            |       |       |
| Preprocessed stdio.h parses              |       |       |
| Preprocessed stdlib.h parses             |       |       |
| Preprocessed string.h parses             |       |       |

ERROR RECOVERY:
| Feature                                  | Test? | Pass? |
|------------------------------------------|-------|-------|
| Missing semicolon → recovers             |       |       |
| Unexpected token → skips to next stmt    |       |       |
| Error in function → next function ok     |       |       |
| Multiple errors collected                |       |       |
| Error in expression → recovers           |       |       |
| Garbage tokens → no panic               |       |       |

DRIVER:
| Feature                                  | Test? | Pass? |
|------------------------------------------|-------|-------|
| forge check file.c (lex+pp+parse)        |       |       |
| forge parse file.c (AST dump)            |       |       |
| forge -E still works                     |       |       |
| Parser diagnostics propagated            |       |       |

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 3 — Edge Case Stress Tests
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Write as tests. No panics, no infinite loops.

1. Empty file → empty TranslationUnit, no errors
2. File with only comments → empty TranslationUnit
3. 50 nested blocks: { { { ... } } } → parses without stack overflow
4. Long expression: a+a+a+...+a (200 terms) → parses
5. Struct with 100 members → parses
6. Function with 50 parameters → parses
7. 20 pointer levels: int **...*x; → parses
8. Nested initializer 10 levels deep: {{{{{{{{{{1}}}}}}}}}} → parses
9. 100 chained function calls: f(g(h(i(j(... → parses or gives clean error
10. Typedef shadowing: typedef int T; { typedef float T; { typedef char T; T x; } }
11. Empty function body: void f() {} → parses
12. Declaration with 20 init-declarators: int a,b,c,d,...; → parses
13. Complex declarator: void (*(*(*fp)(int,int))(double))(char) → parses
14. Every assignment operator in one function → all parse
15. _Generic with 10 type associations → parses

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 4 — Real-World Parse Test
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Test file (tests/lit/parser/mini_program.c):

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
    default:  return (void *)0;
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
    return result == 3 ? 0 : 1;
}
```

Feed through: lex → preprocess → parse. Assert zero errors.

System header parse test:
For each: create temp file with only the include, lex+preprocess+parse, assert 0 errors.
- #include <stddef.h>
- #include <stdint.h>
- #include <limits.h>
- #include <stdio.h>
- #include <stdlib.h>
- #include <string.h>
- #include <errno.h>
- #include <ctype.h>
- #include <assert.h>
- #include <math.h>

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 5 — Performance
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Measure time for full pipeline (lex + preprocess + parse):

Test A: File with #include <stdio.h> + simple main()
  Target: < 200ms debug, < 50ms release

Test B: 10 system headers combined + 50-line program
  Target: < 300ms debug, < 80ms release

Report token count → AST node count ratio.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PART 6 — Final Report
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

1. Code audit results (unwrap count, TODO count, clippy results)
2. Filled completeness matrix
3. Stress test results (all 15)
4. Real-world parse results (mini_program + system headers)
5. Performance numbers
6. Verdict: ready for Phase 4?

Final run:
  cargo test --all → all pass (Phase 0-3 combined)
  cargo clippy --all-targets --all-features -- -D warnings → clean
  cargo fmt --all -- --check → clean

Report total test count.
```

---

## Pitfalls & Debugging Tips

### "typedef int T; T * x;" — the #1 test case
If this parses as multiplication, typedef tracking is broken and ALL real code
will be misparsed. Test this before anything else in every prompt.

### "Declarators are inside-out, left-right"
`int (*fp)(int)` reads: fp is a (pointer to (function taking int returning int)).
Parse: `*` → pointer, `fp` → name, `(int)` → function suffix. The parenthesized
grouping `(*fp)` ensures the pointer binds to fp, not to the return type.

### "Specifier order doesn't matter in C"
`long unsigned int` = `unsigned int long` = `int long unsigned` = `unsigned long`.
Collect as Vec, resolve in sema. Don't try to normalize during parsing.

### "Cast ambiguity requires the typedef table"
`(x)(y)` — function call if x is a variable, cast if x is a typedef.
Without the typedef table, the parser literally cannot decide. This is THE
reason typedef tracking must be flawless.

### "__attribute__ appears in MORE places than you think"
System headers put `__attribute__((visibility("default")))`:
  - Before declarations
  - After declarators
  - On function parameters
  - On struct fields
  - After enum values
  - After function parameter lists
  - On pointer levels (between `*` and identifier)
When in doubt: if you see `__attribute__`, skip it.

### "Error recovery = find the next safe starting point"
Don't try to understand what went wrong. Skip to `;`, `}`, or a type keyword
at the start of a line. Then resume parsing normally.

### "for(int i=0;...) scope"
The declaration `int i` is scoped to the entire for statement (init + body).
Push scope BEFORE parsing init, pop AFTER body. Do NOT let parse_declaration()
push an additional scope — that would double-scope and lose the variable.

---

## Notes

- **Parser does NOT type-check.** `unsigned float x;` is grammatically parseable — the parser
  accepts it, sema (Phase 4) rejects it with "cannot combine 'unsigned' with 'float'".
- **Parser does NOT resolve types.** `[Unsigned, Long, Long]` stays as a Vec in the AST.
- **Parser does NOT evaluate expressions.** `int arr[2+3]` stores `BinaryOp(Add, 2, 3)` as
  the array size, not `5`.
- **K&R function definitions NOT supported.** `int f(x) int x; { }` — deprecated since C89.
  If detected, emit an error diagnostic.
- **GNU statement expressions `({...})` NOT supported.** Emit error if encountered.
