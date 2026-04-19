//! Tests for [`crate::tu::analyze_translation_unit`].
//!
//! Covers the three pieces of behaviour unique to the TU-level entry
//! point:
//!
//! * the handful of builtin typedefs (only `__builtin_va_list` today)
//!   are pre-seeded into the file scope;
//! * every file-scope tentative definition is promoted to a real
//!   definition at end of TU (C17 §6.9.2);
//! * declarations, function definitions, and `_Static_assert` are all
//!   dispatched through the same loop.

use forge_lexer::Span;
use forge_parser::ast::{ExternalDeclaration, StaticAssert, TranslationUnit};

use crate::scope::{Linkage, SymbolKind};
use crate::tu::analyze_translation_unit;

use super::helpers::*;

fn tu(decls: Vec<ExternalDeclaration>) -> TranslationUnit {
    TranslationUnit {
        declarations: decls,
        span: Span::new(0, 0),
    }
}

// ---------------------------------------------------------------------
// Builtin typedef seeding
// ---------------------------------------------------------------------

#[test]
fn builtin_va_list_is_seeded() {
    let (ctx, table) = analyze_translation_unit(&tu(Vec::new()), &ti());
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    let sym = table
        .lookup("__builtin_va_list")
        .expect("__builtin_va_list must be seeded");
    assert!(matches!(sym.kind, SymbolKind::Typedef));
}

// ---------------------------------------------------------------------
// Tentative definition promotion
// ---------------------------------------------------------------------

#[test]
fn file_scope_tentative_int_is_promoted_to_defined() {
    // `int x;` at file scope — tentative, then promoted at end of TU.
    let decl = h_declaration(h_int_specs(), vec![h_init_decl(h_ident_decl("x"), None)]);
    let (ctx, table) =
        analyze_translation_unit(&tu(vec![ExternalDeclaration::Declaration(decl)]), &ti());
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    let sym = table.lookup("x").expect("x must be declared");
    assert!(
        sym.is_defined,
        "tentative definition must be promoted at end of TU"
    );
}

#[test]
fn file_scope_extern_is_not_promoted() {
    // `extern int x;` — never promoted, it is only a declaration.
    let decl = h_declaration(
        h_specs_sc(
            vec![forge_parser::ast::TypeSpecifierToken::Int],
            forge_parser::ast::StorageClass::Extern,
        ),
        vec![h_init_decl(h_ident_decl("x"), None)],
    );
    let (_ctx, table) =
        analyze_translation_unit(&tu(vec![ExternalDeclaration::Declaration(decl)]), &ti());
    let sym = table.lookup("x").expect("x must be declared");
    assert!(!sym.is_defined, "extern declarations are never promoted");
    assert_eq!(sym.linkage, Linkage::External);
}

// ---------------------------------------------------------------------
// ExternalDeclaration dispatch
// ---------------------------------------------------------------------

#[test]
fn function_definition_at_tu_level_registers() {
    let fd = h_fn_int_void("main", vec![h_bstmt(h_return(Some(h_int_lit(0))))]);
    let (ctx, table) =
        analyze_translation_unit(&tu(vec![ExternalDeclaration::FunctionDef(fd)]), &ti());
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    let sym = table.lookup("main").expect("main must exist");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert!(sym.is_defined);
}

#[test]
fn file_scope_static_assert_is_dispatched() {
    let sa = StaticAssert {
        condition: Box::new(h_int_lit(0)),
        message: Some("tu-level fail".into()),
        span: HS,
    };
    let (ctx, _table) =
        analyze_translation_unit(&tu(vec![ExternalDeclaration::StaticAssert(sa)]), &ti());
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("tu-level fail")),
        "expected tu-level static assert to fire, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn empty_translation_unit_has_no_diagnostics() {
    let (ctx, _table) = analyze_translation_unit(&tu(Vec::new()), &ti());
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    assert!(ctx.diagnostics.is_empty(), "{:?}", ctx.diagnostics);
}
