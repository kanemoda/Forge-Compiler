//! Complete C17 AST type hierarchy.
//!
//! This module defines every node the parser can produce.  The types are
//! intentionally *syntactic* — they mirror what the source code says, not
//! what it means.  In particular:
//!
//! * **Type specifiers are a `Vec<TypeSpecifierToken>`**, not a single
//!   resolved type.  `unsigned long long int` produces four entries.
//!   Phase 4 (sema) resolves the combination into a concrete type and
//!   rejects invalid ones like `float double`.
//!
//! * **`StructDef`** is a single type used for *both* `struct` and
//!   `union`.  A [`StructOrUnion`] discriminant tells them apart.
//!
//! * Recursive nodes use `Box<>`.  Arena allocation may be added later
//!   if profiling shows allocation pressure.
//!
//! * Every node that corresponds to a source location carries a
//!   `span: Span` field for diagnostic reporting.

use forge_lexer::{CharPrefix, FloatSuffix, IntSuffix, Span, StringPrefix};

use crate::ast_ops::{AssignOp, BinaryOp, PostfixOp, UnaryOp};

// =========================================================================
// Translation unit
// =========================================================================

/// Root of the AST — a single C translation unit.
#[derive(Clone, Debug)]
pub struct TranslationUnit {
    /// Top-level declarations and function definitions.
    pub declarations: Vec<ExternalDeclaration>,
    /// Span covering the entire translation unit.
    pub span: Span,
}

/// A top-level item: either a function definition or a declaration.
#[derive(Clone, Debug)]
pub enum ExternalDeclaration {
    /// `int main() { ... }`
    FunctionDef(FunctionDef),
    /// `int x;`, `typedef int MyInt;`, `struct Foo { ... };`, etc.
    Declaration(Declaration),
    /// C11 `_Static_assert(cond, "msg");` at file scope.
    StaticAssert(StaticAssert),
}

/// A function definition: specifiers + declarator + compound body.
#[derive(Clone, Debug)]
pub struct FunctionDef {
    /// Return type and storage class.
    pub specifiers: DeclSpecifiers,
    /// Function name and parameter list.
    pub declarator: Declarator,
    /// The function body.
    pub body: CompoundStmt,
    /// Span from the first specifier to the closing `}`.
    pub span: Span,
}

// =========================================================================
// Declarations
// =========================================================================

/// A declaration: specifiers followed by zero or more init-declarators.
///
/// `int x = 1, y;` has specifiers `[Int]` and two init-declarators.
/// `struct Foo { ... };` has specifiers only and no init-declarators.
#[derive(Clone, Debug)]
pub struct Declaration {
    /// Storage class, type specifiers, qualifiers, etc.
    pub specifiers: DeclSpecifiers,
    /// Declarators with optional initialisers.
    pub init_declarators: Vec<InitDeclarator>,
    /// Span covering the full declaration including the trailing `;`.
    pub span: Span,
}

/// The collected specifiers that precede a declarator.
///
/// C allows these in any order (`const static unsigned long int` is the
/// same as `unsigned static const long int`), so they are gathered into
/// vectors/options and validated by sema.
#[derive(Clone, Debug)]
pub struct DeclSpecifiers {
    /// At most one storage class per declaration.
    pub storage_class: Option<StorageClass>,
    /// Primitive type keywords collected in order.  Resolution into a
    /// concrete type is deferred to Phase 4.
    pub type_specifiers: Vec<TypeSpecifierToken>,
    /// `const`, `volatile`, `restrict`, `_Atomic` qualifiers.
    pub type_qualifiers: Vec<TypeQualifier>,
    /// `inline`, `_Noreturn`.
    pub function_specifiers: Vec<FunctionSpecifier>,
    /// `_Alignas(type)` or `_Alignas(expr)`.
    pub alignment: Option<AlignSpec>,
    /// GNU `__attribute__((...))` lists.
    pub attributes: Vec<GnuAttribute>,
    /// Span covering all specifier tokens.
    pub span: Span,
}

/// C17 storage-class specifiers (at most one per declaration).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StorageClass {
    /// `auto`
    Auto,
    /// `register`
    Register,
    /// `static`
    Static,
    /// `extern`
    Extern,
    /// `typedef`
    Typedef,
    /// `_Thread_local`
    ThreadLocal,
}

/// Individual type-specifier tokens as they appear in the source.
///
/// The parser collects these into `DeclSpecifiers::type_specifiers` as a
/// `Vec`.  Sema resolves the combination (e.g. `[Unsigned, Long, Long,
/// Int]` → `unsigned long long`).
#[derive(Clone, Debug)]
pub enum TypeSpecifierToken {
    // -- Primitive keywords --
    /// `void`
    Void,
    /// `char`
    Char,
    /// `short`
    Short,
    /// `int`
    Int,
    /// `long`
    Long,
    /// `float`
    Float,
    /// `double`
    Double,
    /// `signed` / `__signed__`
    Signed,
    /// `unsigned`
    Unsigned,
    /// `_Bool`
    Bool,
    /// `_Complex`
    Complex,

    // -- Compound types --
    /// `struct { ... }` (uses [`StructDef`] with `kind = Struct`)
    Struct(StructDef),
    /// `union { ... }` (uses [`StructDef`] with `kind = Union`)
    Union(StructDef),
    /// `enum { ... }`
    Enum(EnumDef),

    // -- Typedef reference --
    /// A previously-declared typedef name.
    TypedefName(String),

    // -- C11 --
    /// `_Atomic(type-name)`
    Atomic(Box<TypeName>),

    // -- GNU extensions --
    /// `__typeof__(expr)`
    TypeofExpr(Box<Expr>),
    /// `__typeof__(type)`
    TypeofType(Box<TypeName>),
}

/// Type qualifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeQualifier {
    /// `const`
    Const,
    /// `volatile`
    Volatile,
    /// `restrict`
    Restrict,
    /// `_Atomic` (as a qualifier, not a type specifier)
    Atomic,
}

/// Function specifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FunctionSpecifier {
    /// `inline` / `__inline__`
    Inline,
    /// `_Noreturn`
    Noreturn,
}

/// `_Alignas` specifier.
#[derive(Clone, Debug)]
pub enum AlignSpec {
    /// `_Alignas(type-name)`
    AlignAsType(Box<TypeName>),
    /// `_Alignas(constant-expression)`
    AlignAsExpr(Box<Expr>),
}

// =========================================================================
// Declarators
// =========================================================================

/// A declarator with an optional initialiser.
///
/// Used in declaration lists: `int x = 1, y;` produces two of these.
#[derive(Clone, Debug)]
pub struct InitDeclarator {
    /// The declarator (name, pointers, array dims, params).
    pub declarator: Declarator,
    /// Optional initialiser (`= expr` or `= { ... }`).
    pub initializer: Option<Initializer>,
    /// Span covering the declarator and initialiser.
    pub span: Span,
}

/// A declarator: zero or more pointer prefixes followed by a direct
/// declarator.
///
/// The infamous "spiral rule" arises from the recursive interplay
/// between `Declarator` and [`DirectDeclarator`].
#[derive(Clone, Debug)]
pub struct Declarator {
    /// Pointer prefixes, outermost first: `const * volatile *` → two
    /// entries.
    pub pointers: Vec<PointerQualifiers>,
    /// The direct part: identifier, parenthesized, array, or function.
    pub direct: DirectDeclarator,
    /// Span covering the entire declarator.
    pub span: Span,
}

/// Qualifiers attached to one pointer level (`* const volatile`).
#[derive(Clone, Debug)]
pub struct PointerQualifiers {
    /// `const`, `volatile`, `restrict` after the `*`.
    pub qualifiers: Vec<TypeQualifier>,
    /// GNU `__attribute__` can follow `*` in pointer declarators.
    pub attributes: Vec<GnuAttribute>,
}

/// The non-pointer part of a declarator.
#[derive(Clone, Debug)]
pub enum DirectDeclarator {
    /// A plain identifier: `x` in `int x;`.
    Identifier(String, Span),
    /// A parenthesised declarator: `(*fp)` in `int (*fp)(int)`.
    Parenthesized(Box<Declarator>),
    /// An array declarator: `arr[10]`.
    Array {
        /// The declarator to the left of `[`.
        base: Box<DirectDeclarator>,
        /// The size expression, VLA star, or unspecified.
        size: ArraySize,
        /// Qualifiers inside `[static const ...]`.
        qualifiers: Vec<TypeQualifier>,
        /// `true` when `static` appears inside the brackets.
        is_static: bool,
        /// Span covering `[...]`.
        span: Span,
    },
    /// A function declarator: `main(int argc, char **argv)`.
    Function {
        /// The declarator to the left of `(`.
        base: Box<DirectDeclarator>,
        /// Parameter declarations.
        params: Vec<ParamDecl>,
        /// `true` when the parameter list ends with `, ...`.
        is_variadic: bool,
        /// Span covering `(...)`.
        span: Span,
    },
}

/// Array size in a declarator.
#[derive(Clone, Debug)]
pub enum ArraySize {
    /// `int arr[]` — size left unspecified.
    Unspecified,
    /// `int arr[10]` or `int arr[n]` (VLA).
    Expr(Box<Expr>),
    /// `int arr[*]` — VLA with unspecified size (prototype only).
    VLAStar,
}

/// A single parameter declaration inside a function declarator.
#[derive(Clone, Debug)]
pub struct ParamDecl {
    /// Type specifiers and qualifiers.
    pub specifiers: DeclSpecifiers,
    /// `None` for abstract parameters: `void foo(int, int)`.
    pub declarator: Option<Declarator>,
    /// Span covering the parameter.
    pub span: Span,
}

// =========================================================================
// Struct / Union / Enum
// =========================================================================

/// Discriminant for [`StructDef`]: one type serves both `struct` and
/// `union`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StructOrUnion {
    /// `struct`
    Struct,
    /// `union`
    Union,
}

/// A `struct` or `union` definition (or forward declaration).
///
/// Both `struct` and `union` use this single type; the [`kind`](StructDef::kind)
/// field distinguishes them.
#[derive(Clone, Debug)]
pub struct StructDef {
    /// Whether this is a `struct` or `union`.
    pub kind: StructOrUnion,
    /// Tag name, if any.
    pub name: Option<String>,
    /// Member list.  `None` means a forward declaration (`struct foo;`).
    pub members: Option<Vec<StructMember>>,
    /// GNU `__attribute__` on the struct/union tag.
    pub attributes: Vec<GnuAttribute>,
    /// Span covering the entire definition or forward declaration.
    pub span: Span,
}

/// A single member inside a struct/union body.
#[derive(Clone, Debug)]
pub enum StructMember {
    /// A field declaration (possibly with bit-width).
    Field(StructField),
    /// C11 `_Static_assert` inside a struct body.
    StaticAssert(StaticAssert),
}

/// A struct/union field declaration.
///
/// `int x, y:3;` has one `StructField` with two declarators.
#[derive(Clone, Debug)]
pub struct StructField {
    /// Type specifiers and qualifiers for this group of fields.
    pub specifiers: DeclSpecifiers,
    /// Individual field declarators (with optional bit-widths).
    pub declarators: Vec<StructFieldDeclarator>,
    /// Span covering the field declaration.
    pub span: Span,
}

/// One declarator within a struct field declaration, with optional
/// bit-width.
#[derive(Clone, Debug)]
pub struct StructFieldDeclarator {
    /// `None` for an anonymous bit-field: `int : 5;`.
    pub declarator: Option<Declarator>,
    /// Bit-width expression, if this is a bit-field.
    pub bit_width: Option<Box<Expr>>,
    /// Span covering this declarator.
    pub span: Span,
}

/// An `enum` definition (or forward reference).
#[derive(Clone, Debug)]
pub struct EnumDef {
    /// Tag name, if any.
    pub name: Option<String>,
    /// Enumerator list.  `None` means a forward reference.
    pub enumerators: Option<Vec<Enumerator>>,
    /// GNU `__attribute__` on the enum tag.
    pub attributes: Vec<GnuAttribute>,
    /// Span covering the entire definition.
    pub span: Span,
}

/// A single enumerator: `FOO = 42`.
#[derive(Clone, Debug)]
pub struct Enumerator {
    /// Enumerator name.
    pub name: String,
    /// Optional explicit value.
    pub value: Option<Box<Expr>>,
    /// GCC allows `__attribute__` on individual enumerators.
    pub attributes: Vec<GnuAttribute>,
    /// Span covering the enumerator.
    pub span: Span,
}

// =========================================================================
// Initialisers
// =========================================================================

/// An initialiser: either a single expression or a braced list.
#[derive(Clone, Debug)]
pub enum Initializer {
    /// `= expr`
    Expr(Box<Expr>),
    /// `= { .x = 1, [0] = 2, ... }`
    List {
        /// Designated initialisers (designators may be empty).
        items: Vec<DesignatedInit>,
        /// Span covering `{ ... }`.
        span: Span,
    },
}

/// An initialiser with optional designators: `.field = expr` or
/// `[index] = expr`.
#[derive(Clone, Debug)]
pub struct DesignatedInit {
    /// Designator chain.  Empty means no designation.
    pub designators: Vec<Designator>,
    /// The initialiser value.
    pub initializer: Box<Initializer>,
    /// Span covering designators + initialiser.
    pub span: Span,
}

/// A single designator in an initialiser.
#[derive(Clone, Debug)]
pub enum Designator {
    /// `[expr]` — array index.
    Index(Box<Expr>),
    /// `.field` — struct/union member.
    Field(String),
}

// =========================================================================
// Statements
// =========================================================================

/// A compound statement (block): `{ ... }`.
#[derive(Clone, Debug)]
pub struct CompoundStmt {
    /// Block items: declarations, statements, or `_Static_assert`.
    pub items: Vec<BlockItem>,
    /// Span from `{` to `}`.
    pub span: Span,
}

/// An item inside a compound statement.
#[derive(Clone, Debug)]
pub enum BlockItem {
    /// A declaration.
    Declaration(Declaration),
    /// A statement.
    Statement(Stmt),
    /// C11 `_Static_assert` at block scope.
    StaticAssert(StaticAssert),
}

/// A C17 statement.
#[derive(Clone, Debug)]
pub enum Stmt {
    /// `{ ... }`
    Compound(CompoundStmt),
    /// An expression statement (or the empty statement `;`).
    Expr {
        /// `None` for the empty statement `;`.
        expr: Option<Box<Expr>>,
        /// Span covering the expression and its `;`.
        span: Span,
    },
    /// `if (cond) then_branch [else else_branch]`
    If {
        /// Condition expression.
        condition: Box<Expr>,
        /// Taken when true.
        then_branch: Box<Stmt>,
        /// Taken when false (optional).
        else_branch: Option<Box<Stmt>>,
        /// Span from `if` to end of the else branch (or then branch).
        span: Span,
    },
    /// `while (cond) body`
    While {
        /// Loop condition.
        condition: Box<Expr>,
        /// Loop body.
        body: Box<Stmt>,
        /// Span from `while` to end of body.
        span: Span,
    },
    /// `do body while (cond);`
    DoWhile {
        /// Loop body.
        body: Box<Stmt>,
        /// Loop condition.
        condition: Box<Expr>,
        /// Span from `do` to the `;`.
        span: Span,
    },
    /// `for (init; cond; update) body`
    For {
        /// Initialiser: a declaration or expression.
        init: Option<ForInit>,
        /// Loop condition (tested before each iteration).
        condition: Option<Box<Expr>>,
        /// Update expression (after each iteration).
        update: Option<Box<Expr>>,
        /// Loop body.
        body: Box<Stmt>,
        /// Span from `for` to end of body.
        span: Span,
    },
    /// `switch (expr) body`
    Switch {
        /// Controlling expression.
        expr: Box<Expr>,
        /// Body (normally a compound containing `case`/`default`).
        body: Box<Stmt>,
        /// Span from `switch` to end of body.
        span: Span,
    },
    /// `case value: body`
    Case {
        /// Constant expression for this case.
        value: Box<Expr>,
        /// Statement following the case label.
        body: Box<Stmt>,
        /// Span from `case` to end of body.
        span: Span,
    },
    /// `default: body`
    Default {
        /// Statement following the default label.
        body: Box<Stmt>,
        /// Span from `default` to end of body.
        span: Span,
    },
    /// `return [expr];`
    Return {
        /// Optional return value.
        value: Option<Box<Expr>>,
        /// Span from `return` to `;`.
        span: Span,
    },
    /// `break;`
    Break {
        /// Span covering `break;`.
        span: Span,
    },
    /// `continue;`
    Continue {
        /// Span covering `continue;`.
        span: Span,
    },
    /// `goto label;`
    Goto {
        /// Target label name.
        label: String,
        /// Span from `goto` to `;`.
        span: Span,
    },
    /// `label: stmt`
    Label {
        /// Label name.
        name: String,
        /// The labelled statement.
        stmt: Box<Stmt>,
        /// Span from the label to end of the labelled statement.
        span: Span,
    },
}

/// The initialiser clause of a `for` loop.
#[derive(Clone, Debug)]
pub enum ForInit {
    /// `for (int i = 0; ...)` — a declaration.
    Declaration(Declaration),
    /// `for (i = 0; ...)` — an expression.
    Expr(Box<Expr>),
}

/// C11 `_Static_assert(expr, "message")`.
#[derive(Clone, Debug)]
pub struct StaticAssert {
    /// Constant expression that must be non-zero.
    pub condition: Box<Expr>,
    /// Optional string-literal message (C23 makes it optional).
    pub message: Option<String>,
    /// Span covering the entire `_Static_assert(...)`.
    pub span: Span,
}

// =========================================================================
// Expressions
// =========================================================================

/// A C17 expression.
#[derive(Clone, Debug)]
pub enum Expr {
    // -- Literals --
    /// Integer literal: `42`, `0xFF`, `0777`.
    IntLiteral {
        /// Parsed value.
        value: u64,
        /// Optional suffix (`u`, `l`, `ll`, etc.).
        suffix: IntSuffix,
        /// Source span.
        span: Span,
    },
    /// Floating-point literal: `3.14`, `1e10`, `0x1.8p1`.
    FloatLiteral {
        /// Parsed value.
        value: f64,
        /// Optional suffix (`f`, `l`).
        suffix: FloatSuffix,
        /// Source span.
        span: Span,
    },
    /// Character literal: `'a'`, `L'\n'`.
    CharLiteral {
        /// Code-point value.
        value: u32,
        /// Optional prefix (`L`, `u`, `U`).
        prefix: CharPrefix,
        /// Source span.
        span: Span,
    },
    /// String literal: `"hello"`, `u8"utf8"`.
    StringLiteral {
        /// Contents (UTF-8).
        value: String,
        /// Optional prefix (`L`, `u8`, `u`, `U`).
        prefix: StringPrefix,
        /// Source span.
        span: Span,
    },

    // -- Names --
    /// Identifier reference.
    Ident {
        /// Identifier spelling.
        name: String,
        /// Source span.
        span: Span,
    },

    // -- Binary --
    /// Binary operation: `a + b`, `x && y`, etc.
    BinaryOp {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        left: Box<Expr>,
        /// Right operand.
        right: Box<Expr>,
        /// Span covering the entire expression.
        span: Span,
    },

    // -- Unary prefix --
    /// Unary prefix operation: `++x`, `-x`, `!x`, `*p`, `&x`.
    UnaryOp {
        /// The operator.
        op: UnaryOp,
        /// Operand.
        operand: Box<Expr>,
        /// Span covering the entire expression.
        span: Span,
    },

    // -- Unary postfix --
    /// Postfix increment/decrement: `x++`, `x--`.
    PostfixOp {
        /// The operator.
        op: PostfixOp,
        /// Operand.
        operand: Box<Expr>,
        /// Span covering the entire expression.
        span: Span,
    },

    // -- Ternary --
    /// Conditional expression: `a ? b : c`.
    Conditional {
        /// Condition.
        condition: Box<Expr>,
        /// Value when true.
        then_expr: Box<Expr>,
        /// Value when false.
        else_expr: Box<Expr>,
        /// Span covering the entire expression.
        span: Span,
    },

    // -- Assignment --
    /// Assignment: `x = 1`, `x += 2`, etc.
    Assignment {
        /// Assignment operator.
        op: AssignOp,
        /// Left-hand side (must be an lvalue).
        target: Box<Expr>,
        /// Right-hand side.
        value: Box<Expr>,
        /// Span covering the entire expression.
        span: Span,
    },

    // -- Postfix access --
    /// Function call: `foo(a, b)`.
    FunctionCall {
        /// The callee expression.
        callee: Box<Expr>,
        /// Argument expressions.
        args: Vec<Expr>,
        /// Span covering the call.
        span: Span,
    },
    /// Member access: `obj.field` or `ptr->field`.
    MemberAccess {
        /// The object or pointer expression.
        object: Box<Expr>,
        /// Member name.
        member: String,
        /// `true` for `->`, `false` for `.`.
        is_arrow: bool,
        /// Span covering the access.
        span: Span,
    },
    /// Array subscript: `arr[idx]`.
    ArraySubscript {
        /// Array expression.
        array: Box<Expr>,
        /// Index expression.
        index: Box<Expr>,
        /// Span covering the subscript.
        span: Span,
    },

    // -- Type-related --
    /// Cast: `(int)x`.
    Cast {
        /// Target type.
        type_name: Box<TypeName>,
        /// Expression to cast.
        expr: Box<Expr>,
        /// Span covering the cast.
        span: Span,
    },
    /// `sizeof expr` (unparenthesised or parenthesised non-type).
    SizeofExpr {
        /// Expression operand.
        expr: Box<Expr>,
        /// Source span.
        span: Span,
    },
    /// `sizeof(type-name)`.
    SizeofType {
        /// Type operand.
        type_name: Box<TypeName>,
        /// Source span.
        span: Span,
    },
    /// `_Alignof(type-name)`.
    AlignofType {
        /// Type operand.
        type_name: Box<TypeName>,
        /// Source span.
        span: Span,
    },
    /// Compound literal: `(int[]){1, 2, 3}`.
    CompoundLiteral {
        /// Type of the compound literal.
        type_name: Box<TypeName>,
        /// Braced initialiser list.
        initializer: Initializer,
        /// Source span.
        span: Span,
    },

    // -- C11 --
    /// `_Generic(controlling, type: expr, ..., default: expr)`.
    GenericSelection {
        /// Controlling expression.
        controlling: Box<Expr>,
        /// Type associations.
        associations: Vec<GenericAssociation>,
        /// Source span.
        span: Span,
    },

    // -- Comma --
    /// Comma expression: `a, b, c`.
    Comma {
        /// Sub-expressions in evaluation order.
        exprs: Vec<Expr>,
        /// Span covering the entire comma expression.
        span: Span,
    },
}

/// One arm of a `_Generic` selection.
#[derive(Clone, Debug)]
pub struct GenericAssociation {
    /// `None` for the `default:` arm.
    pub type_name: Option<TypeName>,
    /// Result expression for this arm.
    pub expr: Box<Expr>,
    /// Span covering this association.
    pub span: Span,
}

// =========================================================================
// Type names (sizeof, cast, compound literal, _Alignas, _Atomic)
// =========================================================================

/// A type-name: specifiers + optional abstract declarator.
///
/// Used in casts, `sizeof`, compound literals, `_Alignas`, `_Atomic`,
/// and `_Generic` associations.
#[derive(Clone, Debug)]
pub struct TypeName {
    /// Type specifiers and qualifiers.
    pub specifiers: DeclSpecifiers,
    /// Optional abstract declarator (pointers, arrays, function types
    /// without parameter names).
    pub abstract_declarator: Option<AbstractDeclarator>,
    /// Span covering the entire type-name.
    pub span: Span,
}

/// An abstract declarator: pointers and/or a direct abstract part, but
/// no identifier.
#[derive(Clone, Debug)]
pub struct AbstractDeclarator {
    /// Pointer prefixes.
    pub pointers: Vec<PointerQualifiers>,
    /// Optional direct abstract declarator (array/function suffixes).
    pub direct: Option<DirectAbstractDeclarator>,
    /// Span covering the abstract declarator.
    pub span: Span,
}

/// The non-pointer part of an abstract declarator.
#[derive(Clone, Debug)]
pub enum DirectAbstractDeclarator {
    /// `(abstract-declarator)` — parenthesised.
    Parenthesized(Box<AbstractDeclarator>),
    /// Abstract array declarator: `[10]`, `[]`, `[*]`.
    Array {
        /// Base, if this is chained: `[][10]`.
        base: Option<Box<DirectAbstractDeclarator>>,
        /// Size expression, unspecified, or VLA star.
        size: ArraySize,
        /// Span covering `[...]`.
        span: Span,
    },
    /// Abstract function declarator: `(int, int)`.
    Function {
        /// Base, if chained.
        base: Option<Box<DirectAbstractDeclarator>>,
        /// Parameter declarations.
        params: Vec<ParamDecl>,
        /// `true` if variadic.
        is_variadic: bool,
        /// Span covering `(...)`.
        span: Span,
    },
}

// =========================================================================
// GNU extensions
// =========================================================================

/// A single `__attribute__((name(args)))` entry.
#[derive(Clone, Debug)]
pub struct GnuAttribute {
    /// Attribute name (e.g. `noreturn`, `format`, `aligned`).
    pub name: String,
    /// Arguments, if any.  `None` for bare attributes like `noreturn`.
    pub args: Option<Vec<GnuAttributeArg>>,
    /// Span covering this attribute.
    pub span: Span,
}

/// An argument inside a GNU attribute.
#[derive(Clone, Debug)]
pub enum GnuAttributeArg {
    /// A plain identifier argument.
    Ident(String),
    /// An expression argument.
    Expr(Box<Expr>),
    /// Nested form: `format(printf, 1, 2)`.
    Nested {
        /// Name of the nested attribute.
        name: String,
        /// Inner arguments.
        args: Vec<GnuAttributeArg>,
    },
}
