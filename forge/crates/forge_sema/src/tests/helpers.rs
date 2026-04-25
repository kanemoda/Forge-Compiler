//! Shared constructors and fixtures used across `forge_sema` tests.
//!
//! Every test module pulls in these short-hands via
//! `use super::helpers::*;` rather than constructing the full enum
//! variants by hand.

#![allow(dead_code)]

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::{
    BlockItem, CompoundStmt, DeclSpecifiers, Declaration, Declarator, DirectDeclarator, Expr,
    ForInit, FunctionDef, FunctionSpecifier, InitDeclarator, Initializer, ParamDecl, Stmt,
    StorageClass as ParserStorageClass, TypeSpecifierToken,
};
use forge_parser::ast_ops::{AssignOp, BinaryOp};
use forge_parser::node_id::NodeId;
use rustc_hash::FxHashMap;

use crate::stmt::FnContext;
use crate::types::{
    ArraySize, ParamType, QualType, Signedness, StructLayout, StructTypeId, TargetInfo, Type,
    TypeContext, UnionLayout, UnionTypeId,
};

// --------------------------------------------------------------------
// Sentinels
// --------------------------------------------------------------------

/// Zero-length span for hand-built AST nodes.
pub const HS: Span = Span::primary(0, 0);

/// Dummy node id for hand-built AST nodes.
pub const HN: NodeId = NodeId::DUMMY;

// --------------------------------------------------------------------
// Fixtures
// --------------------------------------------------------------------

/// Default x86-64 Linux LP64 target.
pub fn ti() -> TargetInfo {
    TargetInfo::x86_64_linux()
}

/// Empty type context — good enough for tests that never reference a
/// `struct`, `union`, or `enum`.
pub fn ctx() -> TypeContext {
    TypeContext::default()
}

// --------------------------------------------------------------------
// Scalar constructors
// --------------------------------------------------------------------

pub fn void() -> Type {
    Type::Void
}

pub fn t_bool() -> Type {
    Type::Bool
}

pub fn char_plain() -> Type {
    Type::Char {
        signedness: Signedness::Plain,
    }
}

pub fn char_signed() -> Type {
    Type::Char {
        signedness: Signedness::Signed,
    }
}

pub fn char_unsigned() -> Type {
    Type::Char {
        signedness: Signedness::Unsigned,
    }
}

pub fn short() -> Type {
    Type::Short { is_unsigned: false }
}

pub fn ushort() -> Type {
    Type::Short { is_unsigned: true }
}

pub fn int() -> Type {
    Type::Int { is_unsigned: false }
}

pub fn uint() -> Type {
    Type::Int { is_unsigned: true }
}

pub fn long() -> Type {
    Type::Long { is_unsigned: false }
}

pub fn ulong() -> Type {
    Type::Long { is_unsigned: true }
}

pub fn llong() -> Type {
    Type::LongLong { is_unsigned: false }
}

pub fn ullong() -> Type {
    Type::LongLong { is_unsigned: true }
}

pub fn t_float() -> Type {
    Type::Float
}

pub fn t_double() -> Type {
    Type::Double
}

pub fn long_double() -> Type {
    Type::LongDouble
}

// --------------------------------------------------------------------
// Derived constructors
// --------------------------------------------------------------------

pub fn q(ty: Type) -> QualType {
    QualType::unqualified(ty)
}

pub fn ptr_to(qt: QualType) -> Type {
    Type::Pointer {
        pointee: Box::new(qt),
    }
}

pub fn array_of(elem: QualType, size: ArraySize) -> Type {
    Type::Array {
        element: Box::new(elem),
        size,
    }
}

pub fn func(return_ty: QualType, params: Vec<QualType>, is_variadic: bool) -> Type {
    Type::Function {
        return_type: Box::new(return_ty),
        params: params
            .into_iter()
            .map(|ty| ParamType {
                name: None,
                ty,
                has_static_size: false,
            })
            .collect(),
        is_variadic,
        is_prototype: true,
    }
}

pub fn func_noproto(return_ty: QualType) -> Type {
    Type::Function {
        return_type: Box::new(return_ty),
        params: Vec::new(),
        is_variadic: false,
        is_prototype: false,
    }
}

// --------------------------------------------------------------------
// Context helpers
// --------------------------------------------------------------------

/// Register a fully-sized struct with `tag`, `size`, and `align` bytes.
pub fn register_struct(
    ctx: &mut TypeContext,
    id: u32,
    tag: &str,
    size: u64,
    align: u64,
) -> StructTypeId {
    let sid = StructTypeId(id);
    ctx.set_struct(
        sid,
        StructLayout {
            tag: Some(tag.to_string()),
            total_size: size,
            alignment: align,
            is_complete: true,
            ..StructLayout::default()
        },
    );
    sid
}

/// Register a fully-sized union with `tag`, `size`, and `align` bytes.
pub fn register_union(
    ctx: &mut TypeContext,
    id: u32,
    tag: &str,
    size: u64,
    align: u64,
) -> UnionTypeId {
    let uid = UnionTypeId(id);
    ctx.set_union(
        uid,
        UnionLayout {
            tag: Some(tag.to_string()),
            total_size: size,
            alignment: align,
            is_complete: true,
            ..UnionLayout::default()
        },
    );
    uid
}

// --------------------------------------------------------------------
// AST construction helpers — specifiers and declarators
// --------------------------------------------------------------------

/// Minimal `DeclSpecifiers` with only the primitive type tokens set.
pub fn h_specs(ts: Vec<TypeSpecifierToken>) -> DeclSpecifiers {
    DeclSpecifiers {
        storage_class: None,
        type_specifiers: ts,
        type_qualifiers: Vec::new(),
        function_specifiers: Vec::new(),
        alignment: None,
        attributes: Vec::new(),
        span: HS,
    }
}

/// `DeclSpecifiers` with a single storage-class specifier.
pub fn h_specs_sc(ts: Vec<TypeSpecifierToken>, sc: ParserStorageClass) -> DeclSpecifiers {
    let mut s = h_specs(ts);
    s.storage_class = Some(sc);
    s
}

/// `DeclSpecifiers` with extra function specifiers (`inline`, `_Noreturn`).
pub fn h_specs_fnspec(ts: Vec<TypeSpecifierToken>, fs: Vec<FunctionSpecifier>) -> DeclSpecifiers {
    let mut s = h_specs(ts);
    s.function_specifiers = fs;
    s
}

/// `int` specifiers.
pub fn h_int_specs() -> DeclSpecifiers {
    h_specs(vec![TypeSpecifierToken::Int])
}

/// `void` specifiers.
pub fn h_void_specs() -> DeclSpecifiers {
    h_specs(vec![TypeSpecifierToken::Void])
}

/// `DirectDeclarator::Identifier(name)`.
pub fn h_ident(name: &str) -> DirectDeclarator {
    DirectDeclarator::Identifier(name.to_string(), HS)
}

/// Bare `Declarator` wrapping a direct declarator — no pointer prefix.
pub fn h_decl(direct: DirectDeclarator) -> Declarator {
    Declarator {
        pointers: Vec::new(),
        direct,
        span: HS,
    }
}

/// Shorthand for `h_decl(h_ident(name))`.
pub fn h_ident_decl(name: &str) -> Declarator {
    h_decl(h_ident(name))
}

/// `Declarator` for a function named `name` that takes a single `void`
/// parameter (i.e. a prototype with no real parameters).
pub fn h_func_decl_void(name: &str) -> Declarator {
    Declarator {
        pointers: Vec::new(),
        direct: DirectDeclarator::Function {
            base: Box::new(h_ident(name)),
            params: vec![ParamDecl {
                specifiers: h_void_specs(),
                declarator: None,
                span: HS,
                abstract_declarator: None,
            }],
            is_variadic: false,
            span: HS,
        },
        span: HS,
    }
}

/// `Declarator` for `name(int p0, int p1, ...)` — one `int` parameter
/// per entry in `param_names`.  Each parameter keeps its supplied name.
pub fn h_func_decl_int_params(name: &str, param_names: &[&str]) -> Declarator {
    let params: Vec<ParamDecl> = param_names
        .iter()
        .map(|pname| ParamDecl {
            specifiers: h_int_specs(),
            declarator: Some(h_ident_decl(pname)),
            span: HS,
            abstract_declarator: None,
        })
        .collect();
    Declarator {
        pointers: Vec::new(),
        direct: DirectDeclarator::Function {
            base: Box::new(h_ident(name)),
            params,
            is_variadic: false,
            span: HS,
        },
        span: HS,
    }
}

/// `InitDeclarator` pairing a declarator with an optional initialiser.
pub fn h_init_decl(d: Declarator, init: Option<Initializer>) -> InitDeclarator {
    InitDeclarator {
        declarator: d,
        initializer: init,
        span: HS,
        node_id: HN,
    }
}

/// `Declaration` with the given specifiers and init-declarators.
pub fn h_declaration(specifiers: DeclSpecifiers, decls: Vec<InitDeclarator>) -> Declaration {
    Declaration {
        specifiers,
        init_declarators: decls,
        span: HS,
        node_id: HN,
    }
}

// --------------------------------------------------------------------
// AST construction helpers — expressions
// --------------------------------------------------------------------

/// Integer literal expression without a suffix.
pub fn h_int_lit(v: u64) -> Expr {
    Expr::IntLiteral {
        value: v,
        suffix: IntSuffix::None,
        span: HS,
        node_id: HN,
    }
}

/// `Initializer::Expr` wrapping an integer literal.
pub fn h_expr_init(v: u64) -> Initializer {
    Initializer::Expr(Box::new(h_int_lit(v)))
}

/// Identifier expression referencing `name`.
pub fn h_ident_expr(name: &str) -> Expr {
    Expr::Ident {
        name: name.to_string(),
        span: HS,
        node_id: HN,
    }
}

/// `target = value` assignment expression.
pub fn h_assign(target: Expr, value: Expr) -> Expr {
    Expr::Assignment {
        op: AssignOp::Assign,
        target: Box::new(target),
        value: Box::new(value),
        span: HS,
        node_id: HN,
    }
}

/// Binary operation with the given operator.
pub fn h_binop(op: BinaryOp, lhs: Expr, rhs: Expr) -> Expr {
    Expr::BinaryOp {
        op,
        left: Box::new(lhs),
        right: Box::new(rhs),
        span: HS,
        node_id: HN,
    }
}

// --------------------------------------------------------------------
// AST construction helpers — statements and block items
// --------------------------------------------------------------------

/// `BlockItem::Statement` wrapper.
pub fn h_bstmt(s: Stmt) -> BlockItem {
    BlockItem::Statement(s)
}

/// `BlockItem::Declaration` wrapper.
pub fn h_bdecl(d: Declaration) -> BlockItem {
    BlockItem::Declaration(d)
}

/// `CompoundStmt` from a list of block items.
pub fn h_compound(items: Vec<BlockItem>) -> CompoundStmt {
    CompoundStmt { items, span: HS }
}

/// `Stmt::Compound` wrapping a `CompoundStmt` from a list of items.
pub fn h_compound_stmt(items: Vec<BlockItem>) -> Stmt {
    Stmt::Compound(h_compound(items))
}

/// Expression statement wrapping `e`.
pub fn h_expr_stmt(e: Expr) -> Stmt {
    Stmt::Expr {
        expr: Some(Box::new(e)),
        span: HS,
        node_id: HN,
    }
}

/// Empty statement (`;`).
pub fn h_empty_stmt() -> Stmt {
    Stmt::Expr {
        expr: None,
        span: HS,
        node_id: HN,
    }
}

/// `return;` or `return <expr>;`.
pub fn h_return(value: Option<Expr>) -> Stmt {
    Stmt::Return {
        value: value.map(Box::new),
        span: HS,
        node_id: HN,
    }
}

/// `break;`
pub fn h_break() -> Stmt {
    Stmt::Break {
        span: HS,
        node_id: HN,
    }
}

/// `continue;`
pub fn h_continue() -> Stmt {
    Stmt::Continue {
        span: HS,
        node_id: HN,
    }
}

/// `goto label;`
pub fn h_goto(label: &str) -> Stmt {
    Stmt::Goto {
        label: label.to_string(),
        span: HS,
        node_id: HN,
    }
}

/// `label: <inner>`
pub fn h_label(name: &str, inner: Stmt) -> Stmt {
    Stmt::Label {
        name: name.to_string(),
        stmt: Box::new(inner),
        span: HS,
        node_id: HN,
    }
}

/// `if (cond) then_branch [else else_branch]`
pub fn h_if(cond: Expr, then_branch: Stmt, else_branch: Option<Stmt>) -> Stmt {
    Stmt::If {
        condition: Box::new(cond),
        then_branch: Box::new(then_branch),
        else_branch: else_branch.map(Box::new),
        span: HS,
        node_id: HN,
    }
}

/// `while (cond) body`
pub fn h_while(cond: Expr, body: Stmt) -> Stmt {
    Stmt::While {
        condition: Box::new(cond),
        body: Box::new(body),
        span: HS,
        node_id: HN,
    }
}

/// `do body while (cond);`
pub fn h_do_while(body: Stmt, cond: Expr) -> Stmt {
    Stmt::DoWhile {
        body: Box::new(body),
        condition: Box::new(cond),
        span: HS,
        node_id: HN,
    }
}

/// `for (init; cond; update) body`
pub fn h_for(init: Option<ForInit>, cond: Option<Expr>, update: Option<Expr>, body: Stmt) -> Stmt {
    Stmt::For {
        init,
        condition: cond.map(Box::new),
        update: update.map(Box::new),
        body: Box::new(body),
        span: HS,
        node_id: HN,
    }
}

/// `switch (expr) body`
pub fn h_switch(expr: Expr, body: Stmt) -> Stmt {
    Stmt::Switch {
        expr: Box::new(expr),
        body: Box::new(body),
        span: HS,
        node_id: HN,
    }
}

/// `case value: body`
pub fn h_case(value: Expr, body: Stmt) -> Stmt {
    Stmt::Case {
        value: Box::new(value),
        body: Box::new(body),
        span: HS,
        node_id: HN,
    }
}

/// `default: body`
pub fn h_default(body: Stmt) -> Stmt {
    Stmt::Default {
        body: Box::new(body),
        span: HS,
        node_id: HN,
    }
}

// --------------------------------------------------------------------
// AST construction helpers — function definitions
// --------------------------------------------------------------------

/// Build a `FunctionDef` from its three syntactic pieces.
pub fn h_fn_def(
    specifiers: DeclSpecifiers,
    declarator: Declarator,
    body: CompoundStmt,
) -> FunctionDef {
    FunctionDef {
        specifiers,
        declarator,
        body,
        span: HS,
        node_id: HN,
    }
}

/// `int name(void) { body }` function definition.
pub fn h_fn_int_void(name: &str, body: Vec<BlockItem>) -> FunctionDef {
    h_fn_def(h_int_specs(), h_func_decl_void(name), h_compound(body))
}

/// `void name(void) { body }` function definition.
pub fn h_fn_void_void(name: &str, body: Vec<BlockItem>) -> FunctionDef {
    h_fn_def(h_void_specs(), h_func_decl_void(name), h_compound(body))
}

// --------------------------------------------------------------------
// FnContext builder
// --------------------------------------------------------------------

/// Construct a minimal [`FnContext`] with the given return type and no
/// surrounding loop/switch nesting.  Used by tests that drive
/// [`crate::stmt::analyze_stmt`] directly rather than through
/// [`crate::stmt::analyze_function_def`].
pub fn fn_ctx(return_ty: QualType) -> FnContext {
    FnContext {
        return_type: return_ty,
        in_loop: false,
        in_switch: false,
        switch_stack: Vec::new(),
        labels_defined: FxHashMap::default(),
        labels_referenced: Vec::new(),
        return_seen: false,
        is_noreturn: false,
    }
}

// --------------------------------------------------------------------
// End-to-end lex + parse + sema helpers
// --------------------------------------------------------------------

use forge_diagnostics::{Diagnostic, FileId, Severity};
use forge_lexer::Lexer;
use forge_parser::Parser;

use crate::context::SemaContext;
use crate::scope::SymbolTable;
use crate::tu::analyze_translation_unit;

/// Lex, parse, and run sema on `src`, returning every diagnostic
/// emitted by any phase plus the final context and symbol table.
///
/// No preprocessor is involved — sources must be self-contained C17
/// fragments that the lexer and parser can consume directly.
pub fn analyze_source(src: &str) -> (Vec<Diagnostic>, SemaContext, SymbolTable) {
    let tokens = Lexer::new(src, FileId::PRIMARY).tokenize();
    let (tu, parse_diags) = Parser::parse(tokens);
    let target = ti();
    let (ctx, table) = analyze_translation_unit(&tu, &target);
    let mut all = parse_diags;
    all.extend(ctx.diagnostics.iter().cloned());
    (all, ctx, table)
}

/// Assert that `src` lexes, parses, and sema-analyses without a single
/// error-severity diagnostic.
pub fn assert_source_clean(src: &str) {
    let (diags, _ctx, _table) = analyze_source(src);
    let errors: Vec<&Diagnostic> = diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Error))
        .collect();
    assert!(
        errors.is_empty(),
        "expected no errors, got: {errors:?}\n\nfull source:\n{src}"
    );
}

/// Assert that `src` produces at least one error-severity diagnostic
/// when lexed, parsed, and sema-analysed.
pub fn assert_source_has_errors(src: &str) {
    let (diags, _ctx, _table) = analyze_source(src);
    let has_error = diags.iter().any(|d| matches!(d.severity, Severity::Error));
    assert!(
        has_error,
        "expected at least one error, got none\n\nfull source:\n{src}"
    );
}
