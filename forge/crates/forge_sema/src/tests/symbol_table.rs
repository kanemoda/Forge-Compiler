//! Tests for the symbol table and scope machinery.
//!
//! Covers declare/lookup, shadowing across nested scopes, duplicate
//! detection, `extern` merging, tag/ordinary namespace independence,
//! and forward-declared struct completion.

use forge_lexer::Span;

use crate::context::SemaContext;
use crate::scope::{Linkage, ScopeKind, StorageClass, Symbol, SymbolKind, SymbolTable, TagEntry};
use crate::types::{QualType, StructLayout, StructTypeId, UnionTypeId};

use super::helpers::{int, q, uint};

const S: Span = Span::primary(0, 0);

fn sym(name: &str, ty: QualType, kind: SymbolKind) -> Symbol {
    Symbol {
        id: 0,
        name: name.to_string(),
        ty,
        kind,
        storage: StorageClass::None,
        linkage: Linkage::None,
        span: S,
        is_defined: true,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
        address_taken: false,
    }
}

fn sym_extern(name: &str, ty: QualType) -> Symbol {
    Symbol {
        id: 0,
        name: name.to_string(),
        ty,
        kind: SymbolKind::Variable,
        storage: StorageClass::Extern,
        linkage: Linkage::External,
        span: S,
        is_defined: false,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
        address_taken: false,
    }
}

// ---------------------------------------------------------------------
// Basic declare / lookup
// ---------------------------------------------------------------------

#[test]
fn fresh_table_has_a_single_file_scope() {
    let table = SymbolTable::new();
    assert_eq!(table.scope_depth(), 1);
    assert_eq!(table.current_scope_kind(), ScopeKind::File);
}

#[test]
fn declare_then_lookup_returns_the_symbol() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let id = table
        .declare(sym("x", q(int()), SymbolKind::Variable), &mut ctx)
        .expect("declare");
    let found = table.lookup("x").expect("lookup");
    assert_eq!(found.id, id);
    assert_eq!(found.name, "x");
    assert!(!ctx.has_errors());
}

#[test]
fn lookup_missing_name_returns_none() {
    let table = SymbolTable::new();
    assert!(table.lookup("nothing").is_none());
}

// ---------------------------------------------------------------------
// Shadowing and scope pop
// ---------------------------------------------------------------------

#[test]
fn inner_scope_shadows_outer_scope() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table
        .declare(sym("x", q(int()), SymbolKind::Variable), &mut ctx)
        .unwrap();

    table.push_scope(ScopeKind::Block);
    table
        .declare(sym("x", q(uint()), SymbolKind::Variable), &mut ctx)
        .unwrap();

    let inner = table.lookup("x").unwrap();
    assert_eq!(inner.ty, q(uint()));
    assert!(!ctx.has_errors());
}

#[test]
fn popping_inner_scope_restores_outer_symbol() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table
        .declare(sym("x", q(int()), SymbolKind::Variable), &mut ctx)
        .unwrap();

    table.push_scope(ScopeKind::Block);
    table
        .declare(sym("x", q(uint()), SymbolKind::Variable), &mut ctx)
        .unwrap();
    table.pop_scope();

    let outer = table.lookup("x").unwrap();
    assert_eq!(outer.ty, q(int()));
}

#[test]
#[should_panic(expected = "pop_scope underflow")]
fn pop_file_scope_panics() {
    let mut table = SymbolTable::new();
    table.pop_scope();
}

// ---------------------------------------------------------------------
// Duplicate detection
// ---------------------------------------------------------------------

#[test]
fn same_scope_duplicate_definition_is_an_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table
        .declare(sym("x", q(int()), SymbolKind::Variable), &mut ctx)
        .unwrap();
    let second = table.declare(sym("x", q(int()), SymbolKind::Variable), &mut ctx);
    assert!(second.is_none());
    assert!(ctx.has_errors());
}

#[test]
fn kind_mismatch_is_an_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table
        .declare(sym("x", q(int()), SymbolKind::Variable), &mut ctx)
        .unwrap();
    let second = table.declare(sym("x", q(int()), SymbolKind::Typedef), &mut ctx);
    assert!(second.is_none());
    assert!(ctx.has_errors());
}

#[test]
fn incompatible_redeclaration_is_an_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table.declare(sym_extern("x", q(int())), &mut ctx).unwrap();
    // Try to redeclare as a different arithmetic type — incompatible.
    let second = table.declare(sym_extern("x", q(uint())), &mut ctx);
    assert!(second.is_none());
    assert!(ctx.has_errors());
}

// ---------------------------------------------------------------------
// Extern merging
// ---------------------------------------------------------------------

#[test]
fn two_compatible_externs_merge_into_one_symbol() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let first = table.declare(sym_extern("x", q(int())), &mut ctx).unwrap();
    let second = table.declare(sym_extern("x", q(int())), &mut ctx).unwrap();
    assert_eq!(first, second);
    assert_eq!(table.symbol_count(), 1);
    assert!(!ctx.has_errors());
}

#[test]
fn duplicate_definitions_are_rejected_even_when_compatible() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let s1 = sym("x", q(int()), SymbolKind::Variable);
    let s2 = sym("x", q(int()), SymbolKind::Variable); // both is_defined = true
    table.declare(s1, &mut ctx).unwrap();
    assert!(table.declare(s2, &mut ctx).is_none());
    assert!(ctx.has_errors());
}

// ---------------------------------------------------------------------
// Tag / ordinary independence
// ---------------------------------------------------------------------

#[test]
fn tag_and_ordinary_namespaces_are_independent() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let sid = ctx.type_ctx.fresh_struct_id();
    ctx.type_ctx.set_struct(
        sid,
        StructLayout {
            tag: Some("foo".into()),
            ..StructLayout::default()
        },
    );

    let tag = table
        .declare_tag("foo", TagEntry::Struct(sid), S, &mut ctx)
        .expect("declare_tag");
    let ord = table
        .declare(sym("foo", q(int()), SymbolKind::Variable), &mut ctx)
        .expect("declare variable");

    // Both independently resolvable.
    let (tid, entry) = table.lookup_tag("foo").unwrap();
    assert_eq!(tid, tag);
    assert!(matches!(entry, TagEntry::Struct(_)));

    let variable = table.lookup("foo").unwrap();
    assert_eq!(variable.id, ord);
    assert_eq!(variable.ty, q(int()));

    assert!(!ctx.has_errors());
}

#[test]
fn redeclaring_same_struct_tag_returns_same_id() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let sid = ctx.type_ctx.fresh_struct_id();
    let first = table
        .declare_tag("pair", TagEntry::Struct(sid), S, &mut ctx)
        .unwrap();
    let second = table
        .declare_tag("pair", TagEntry::Struct(sid), S, &mut ctx)
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(table.tag_count(), 1);
    assert!(!ctx.has_errors());
}

#[test]
fn redeclaring_tag_with_different_kind_is_an_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let sid = ctx.type_ctx.fresh_struct_id();
    table
        .declare_tag("foo", TagEntry::Struct(sid), S, &mut ctx)
        .unwrap();

    let result = table.declare_tag("foo", TagEntry::Union(UnionTypeId(0)), S, &mut ctx);
    assert!(result.is_none());
    assert!(ctx.has_errors());
}

#[test]
fn forward_struct_declaration_then_completion_reuses_the_id() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // `struct list;` — forward declaration.
    let sid = ctx.type_ctx.fresh_struct_id();
    let first = table
        .declare_tag("list", TagEntry::Struct(sid), S, &mut ctx)
        .unwrap();

    // Later: `struct list { int x; };` — register layout on the same id.
    ctx.type_ctx.set_struct(
        sid,
        StructLayout {
            tag: Some("list".into()),
            total_size: 4,
            alignment: 4,
            is_complete: true,
            ..StructLayout::default()
        },
    );

    // Declaring the tag with the same id again is a re-reference, not a
    // new tag.
    let second = table
        .declare_tag("list", TagEntry::Struct(sid), S, &mut ctx)
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(table.tag_count(), 1);
    assert!(!ctx.has_errors());
}

#[test]
fn inner_scope_can_reuse_outer_tag_name_with_a_different_kind() {
    // C17 allows hiding tags via an inner scope: `struct foo` at file
    // scope and `union foo` inside a block are unrelated types.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let sid = ctx.type_ctx.fresh_struct_id();
    table
        .declare_tag("foo", TagEntry::Struct(sid), S, &mut ctx)
        .unwrap();

    table.push_scope(ScopeKind::Block);
    let uid = ctx.type_ctx.fresh_union_id();
    let inner = table.declare_tag("foo", TagEntry::Union(uid), S, &mut ctx);
    assert!(inner.is_some(), "inner scope should hide the outer tag");
    assert!(!ctx.has_errors());

    table.pop_scope();

    // Outer scope still sees the struct.
    let (_, entry) = table.lookup_tag("foo").unwrap();
    assert!(matches!(entry, TagEntry::Struct(_)));
}

#[test]
fn lookup_in_current_scope_does_not_walk_outward() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table
        .declare(sym("x", q(int()), SymbolKind::Variable), &mut ctx)
        .unwrap();

    table.push_scope(ScopeKind::Block);
    assert!(table.lookup_in_current_scope("x").is_none());
    // Outer walk still finds it.
    assert!(table.lookup("x").is_some());
}

#[test]
fn symbol_id_is_assigned_and_dense() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let a = table
        .declare(sym("a", q(int()), SymbolKind::Variable), &mut ctx)
        .unwrap();
    let b = table
        .declare(sym("b", q(int()), SymbolKind::Variable), &mut ctx)
        .unwrap();
    assert_eq!(a, 0);
    assert_eq!(b, 1);
    // The stored symbol has its id stamped.
    assert_eq!(table.symbol(a).id, a);
    assert_eq!(table.symbol(b).id, b);
    let _ = StructTypeId(0); // silence unused-import concerns
}
