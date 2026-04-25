//! Tests for [`analyze_declaration`] and [`analyze_static_assert`].
//!
//! These cover the linkage / storage-class / function-specifier logic —
//! tentative definitions at file scope, static → internal linkage,
//! block-scope extern, function declarations with `inline` / `_Noreturn`,
//! typedef declarations, and `_Static_assert` success / failure.
//!
//! Initialiser-specific behaviour (incomplete-array refinement, excess
//! elements, designator shape, etc.) lives in the companion
//! `initializers` module.

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::{
    DeclSpecifiers, Declaration, Declarator, DirectDeclarator, Expr, FunctionSpecifier,
    InitDeclarator, Initializer, ParamDecl, StaticAssert, StorageClass as ParserStorageClass,
    TypeSpecifierToken,
};
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::declare::{analyze_declaration, analyze_static_assert};
use crate::scope::{Linkage, ScopeKind, StorageClass, SymbolKind, SymbolTable};

use super::helpers::ti;

const S: Span = Span::primary(0, 0);
const N: NodeId = NodeId::DUMMY;

// ---------------------------------------------------------------------
// Construction helpers
// ---------------------------------------------------------------------

fn specs(ts: Vec<TypeSpecifierToken>) -> DeclSpecifiers {
    DeclSpecifiers {
        storage_class: None,
        type_specifiers: ts,
        type_qualifiers: Vec::new(),
        function_specifiers: Vec::new(),
        alignment: None,
        attributes: Vec::new(),
        span: S,
    }
}

fn specs_sc(ts: Vec<TypeSpecifierToken>, sc: ParserStorageClass) -> DeclSpecifiers {
    let mut s = specs(ts);
    s.storage_class = Some(sc);
    s
}

fn specs_fnspec(ts: Vec<TypeSpecifierToken>, fs: Vec<FunctionSpecifier>) -> DeclSpecifiers {
    let mut s = specs(ts);
    s.function_specifiers = fs;
    s
}

fn ident(name: &str) -> DirectDeclarator {
    DirectDeclarator::Identifier(name.to_string(), S)
}

fn decl(direct: DirectDeclarator) -> Declarator {
    Declarator {
        pointers: Vec::new(),
        direct,
        span: S,
    }
}

fn func_decl(name: &str) -> Declarator {
    Declarator {
        pointers: Vec::new(),
        direct: DirectDeclarator::Function {
            base: Box::new(ident(name)),
            params: Vec::new(),
            is_variadic: false,
            span: S,
        },
        span: S,
    }
}

fn func_decl_void(name: &str) -> Declarator {
    Declarator {
        pointers: Vec::new(),
        direct: DirectDeclarator::Function {
            base: Box::new(ident(name)),
            params: vec![ParamDecl {
                specifiers: specs(vec![TypeSpecifierToken::Void]),
                declarator: None,
                span: S,
                abstract_declarator: None,
            }],
            is_variadic: false,
            span: S,
        },
        span: S,
    }
}

fn init_decl(d: Declarator, init: Option<Initializer>) -> InitDeclarator {
    InitDeclarator {
        declarator: d,
        initializer: init,
        span: S,
        node_id: N,
    }
}

fn declaration(specifiers: DeclSpecifiers, decls: Vec<InitDeclarator>) -> Declaration {
    Declaration {
        specifiers,
        init_declarators: decls,
        span: S,
        node_id: N,
    }
}

fn int_lit(v: u64) -> Expr {
    Expr::IntLiteral {
        value: v,
        suffix: IntSuffix::None,
        span: S,
        node_id: N,
    }
}

fn expr_init(v: u64) -> Initializer {
    Initializer::Expr(Box::new(int_lit(v)))
}

// ---------------------------------------------------------------------
// File-scope variable declarations
// ---------------------------------------------------------------------

#[test]
fn file_scope_int_x_is_tentative_external() {
    // `int x;` at file scope → tentative external object.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let d = declaration(
        specs(vec![TypeSpecifierToken::Int]),
        vec![init_decl(decl(ident("x")), None)],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let sym = table.lookup("x").expect("x must be declared");
    assert_eq!(sym.kind, SymbolKind::Variable);
    assert_eq!(sym.storage, StorageClass::None);
    assert_eq!(sym.linkage, Linkage::External);
    assert!(
        !sym.is_defined,
        "tentative at file scope is not a definition"
    );
}

#[test]
fn file_scope_int_x_with_initializer_is_defined() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let d = declaration(
        specs(vec![TypeSpecifierToken::Int]),
        vec![init_decl(decl(ident("x")), Some(expr_init(5)))],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let sym = table.lookup("x").expect("x must be declared");
    assert!(sym.is_defined);
    assert_eq!(sym.linkage, Linkage::External);
}

#[test]
fn file_scope_static_int_x_is_internal() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let d = declaration(
        specs_sc(vec![TypeSpecifierToken::Int], ParserStorageClass::Static),
        vec![init_decl(decl(ident("x")), None)],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let sym = table.lookup("x").expect("x must be declared");
    assert_eq!(sym.storage, StorageClass::Static);
    assert_eq!(sym.linkage, Linkage::Internal);
}

#[test]
fn file_scope_extern_int_x_is_external_not_defined() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let d = declaration(
        specs_sc(vec![TypeSpecifierToken::Int], ParserStorageClass::Extern),
        vec![init_decl(decl(ident("x")), None)],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let sym = table.lookup("x").expect("x must be declared");
    assert_eq!(sym.storage, StorageClass::Extern);
    assert_eq!(sym.linkage, Linkage::External);
    assert!(!sym.is_defined);
}

// ---------------------------------------------------------------------
// Block-scope variable declarations
// ---------------------------------------------------------------------

#[test]
fn block_scope_int_x_is_no_linkage_defined() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table.push_scope(ScopeKind::Block);

    let d = declaration(
        specs(vec![TypeSpecifierToken::Int]),
        vec![init_decl(decl(ident("x")), None)],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let sym = table.lookup("x").expect("x must be declared");
    assert_eq!(sym.linkage, Linkage::None);
    assert!(
        sym.is_defined,
        "auto objects are defined at their declaration"
    );
}

#[test]
fn block_scope_extern_int_x_is_external_not_defined() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table.push_scope(ScopeKind::Block);

    let d = declaration(
        specs_sc(vec![TypeSpecifierToken::Int], ParserStorageClass::Extern),
        vec![init_decl(decl(ident("x")), None)],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let sym = table.lookup("x").expect("x must be declared");
    assert_eq!(sym.linkage, Linkage::External);
    assert!(!sym.is_defined);
}

// ---------------------------------------------------------------------
// Function declarations
// ---------------------------------------------------------------------

#[test]
fn function_decl_has_external_linkage_not_defined() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let d = declaration(
        specs(vec![TypeSpecifierToken::Int]),
        vec![init_decl(func_decl_void("foo"), None)],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let sym = table.lookup("foo").expect("foo must be declared");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert_eq!(sym.linkage, Linkage::External);
    assert!(!sym.is_defined, "a bare declaration is not a definition");
    assert!(!sym.is_inline);
    assert!(!sym.is_noreturn);
}

#[test]
fn function_decl_inline_sets_is_inline() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let d = declaration(
        specs_fnspec(
            vec![TypeSpecifierToken::Int],
            vec![FunctionSpecifier::Inline],
        ),
        vec![init_decl(func_decl_void("foo"), None)],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let sym = table.lookup("foo").expect("foo must be declared");
    assert!(sym.is_inline);
    assert!(!sym.is_noreturn);
}

#[test]
fn function_decl_noreturn_sets_is_noreturn() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let d = declaration(
        specs_fnspec(
            vec![TypeSpecifierToken::Void],
            vec![FunctionSpecifier::Noreturn],
        ),
        vec![init_decl(func_decl_void("die"), None)],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let sym = table.lookup("die").expect("die must be declared");
    assert!(sym.is_noreturn);
}

#[test]
fn block_scope_function_declaration_has_external_linkage() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table.push_scope(ScopeKind::Block);

    let d = declaration(
        specs(vec![TypeSpecifierToken::Int]),
        vec![init_decl(func_decl("foo"), None)],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let sym = table.lookup("foo").expect("foo must be declared");
    assert_eq!(sym.linkage, Linkage::External);
}

#[test]
fn function_with_initializer_errors() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let d = declaration(
        specs(vec![TypeSpecifierToken::Int]),
        vec![init_decl(func_decl_void("foo"), Some(expr_init(0)))],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.has_errors(),
        "expected error for function with initializer"
    );
}

// ---------------------------------------------------------------------
// Typedef declarations
// ---------------------------------------------------------------------

#[test]
fn typedef_declaration_registers_symbol() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let d = declaration(
        specs_sc(vec![TypeSpecifierToken::Int], ParserStorageClass::Typedef),
        vec![init_decl(decl(ident("MyInt")), None)],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let sym = table.lookup("MyInt").expect("MyInt must be declared");
    assert!(matches!(sym.kind, SymbolKind::Typedef));
    assert_eq!(sym.linkage, Linkage::None);
}

#[test]
fn typedef_with_initializer_errors() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let d = declaration(
        specs_sc(vec![TypeSpecifierToken::Int], ParserStorageClass::Typedef),
        vec![init_decl(decl(ident("MyInt")), Some(expr_init(0)))],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.has_errors(),
        "expected error for typedef with initializer"
    );
}

// ---------------------------------------------------------------------
// Multiple init-declarators in one declaration
// ---------------------------------------------------------------------

#[test]
fn multiple_init_declarators_all_registered() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // `int a, b = 5, c;`
    let d = declaration(
        specs(vec![TypeSpecifierToken::Int]),
        vec![
            init_decl(decl(ident("a")), None),
            init_decl(decl(ident("b")), Some(expr_init(5))),
            init_decl(decl(ident("c")), None),
        ],
    );
    analyze_declaration(&d, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    assert!(table.lookup("a").is_some());
    let b = table.lookup("b").expect("b must be declared");
    assert!(b.is_defined);
    assert!(table.lookup("c").is_some());
}

// ---------------------------------------------------------------------
// Redeclaration semantics
// ---------------------------------------------------------------------

#[test]
fn redeclaration_with_compatible_type_merges() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let d1 = declaration(
        specs_sc(vec![TypeSpecifierToken::Int], ParserStorageClass::Extern),
        vec![init_decl(decl(ident("x")), None)],
    );
    analyze_declaration(&d1, &mut table, &ti(), &mut ctx);

    let d2 = declaration(
        specs(vec![TypeSpecifierToken::Int]),
        vec![init_decl(decl(ident("x")), Some(expr_init(7)))],
    );
    analyze_declaration(&d2, &mut table, &ti(), &mut ctx);

    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    let sym = table.lookup("x").expect("x must be declared");
    assert!(sym.is_defined);
}

// ---------------------------------------------------------------------
// _Static_assert
// ---------------------------------------------------------------------

#[test]
fn static_assert_true_is_silent() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let sa = StaticAssert {
        condition: Box::new(int_lit(1)),
        message: Some("must be true".into()),
        span: S,
    };
    analyze_static_assert(&sa, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn static_assert_false_emits_error_with_message() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let sa = StaticAssert {
        condition: Box::new(int_lit(0)),
        message: Some("boom".into()),
        span: S,
    };
    analyze_static_assert(&sa, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(
        ctx.diagnostics.iter().any(|d| d.message.contains("boom")),
        "expected message 'boom' in diagnostics, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn static_assert_false_without_message_uses_default() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let sa = StaticAssert {
        condition: Box::new(int_lit(0)),
        message: None,
        span: S,
    };
    analyze_static_assert(&sa, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("static assertion failed")),
        "expected default message, got {:?}",
        ctx.diagnostics
    );
}
