//! Tests for [`resolve_type_specifiers`].
//!
//! We exercise every legal primitive combination, every major error
//! mode, `struct` / `union` / `enum` reference, typedef lookup, and the
//! interaction between type qualifiers and the resulting [`QualType`].

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::{
    AlignSpec, DeclSpecifiers, Expr, TypeName, TypeQualifier, TypeSpecifierToken,
};
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::resolve::resolve_type_specifiers;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{QualType, Type};

use super::helpers::{
    char_plain, char_signed, char_unsigned, int, llong, long, long_double, q, short, t_bool,
    t_double, t_float, ti, uint, ullong, ulong, ushort, void,
};

const S: Span = Span::new(0, 0);
const N: NodeId = NodeId::DUMMY;

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

fn specs_with_quals(ts: Vec<TypeSpecifierToken>, quals: Vec<TypeQualifier>) -> DeclSpecifiers {
    DeclSpecifiers {
        storage_class: None,
        type_specifiers: ts,
        type_qualifiers: quals,
        function_specifiers: Vec::new(),
        alignment: None,
        attributes: Vec::new(),
        span: S,
    }
}

fn specs_with_alignas(ts: Vec<TypeSpecifierToken>, align: AlignSpec) -> DeclSpecifiers {
    DeclSpecifiers {
        storage_class: None,
        type_specifiers: ts,
        type_qualifiers: Vec::new(),
        function_specifiers: Vec::new(),
        alignment: Some(align),
        attributes: Vec::new(),
        span: S,
    }
}

fn resolve_ok(ts: Vec<TypeSpecifierToken>) -> QualType {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let qt = resolve_type_specifiers(&specs(ts), &mut table, &ti(), &mut ctx)
        .expect("resolve_type_specifiers must succeed");
    assert!(
        !ctx.has_errors(),
        "unexpected diagnostics: {:?}",
        ctx.diagnostics
    );
    qt
}

fn resolve_err(ts: Vec<TypeSpecifierToken>) {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let result = resolve_type_specifiers(&specs(ts), &mut table, &ti(), &mut ctx);
    assert!(
        result.is_none() || ctx.has_errors(),
        "expected either None or at least one diagnostic"
    );
    assert!(ctx.has_errors(), "expected a diagnostic");
}

// ---------------------------------------------------------------------
// Integer primitives
// ---------------------------------------------------------------------

#[test]
fn int_specifier() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Int]).ty, int());
}

#[test]
fn unsigned_int_specifier() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Unsigned, Int]).ty, uint());
}

#[test]
fn bare_unsigned_is_unsigned_int() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Unsigned]).ty, uint());
}

#[test]
fn bare_signed_is_int() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Signed]).ty, int());
}

#[test]
fn long_int() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Long, Int]).ty, long());
}

#[test]
fn bare_long_is_long() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Long]).ty, long());
}

#[test]
fn long_long_is_long_long() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Long, Long]).ty, llong());
}

#[test]
fn unsigned_long_long_int() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Unsigned, Long, Long, Int]).ty, ullong());
}

#[test]
fn specifiers_are_order_independent() {
    use TypeSpecifierToken::*;
    // `long unsigned int long` — same tokens in an unusual order.
    assert_eq!(resolve_ok(vec![Long, Unsigned, Int, Long]).ty, ullong());
}

#[test]
fn unsigned_long_is_ulong() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Unsigned, Long]).ty, ulong());
}

#[test]
fn short_is_short() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Short]).ty, short());
}

#[test]
fn short_int_is_short() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Short, Int]).ty, short());
}

#[test]
fn unsigned_short_is_ushort() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Unsigned, Short]).ty, ushort());
}

#[test]
fn plain_char_is_plain() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Char]).ty, char_plain());
}

#[test]
fn signed_char_is_signed() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Signed, Char]).ty, char_signed());
}

#[test]
fn unsigned_char_is_unsigned() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Unsigned, Char]).ty, char_unsigned());
}

// ---------------------------------------------------------------------
// Floating point
// ---------------------------------------------------------------------

#[test]
fn float_is_float() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Float]).ty, t_float());
}

#[test]
fn double_is_double() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Double]).ty, t_double());
}

#[test]
fn long_double_is_long_double() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Long, Double]).ty, long_double());
}

// ---------------------------------------------------------------------
// Bool / Void
// ---------------------------------------------------------------------

#[test]
fn bool_is_bool() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Bool]).ty, t_bool());
}

#[test]
fn void_is_void() {
    use TypeSpecifierToken::*;
    assert_eq!(resolve_ok(vec![Void]).ty, void());
}

// ---------------------------------------------------------------------
// Qualifier application
// ---------------------------------------------------------------------

#[test]
fn const_int_is_qualified() {
    use TypeSpecifierToken::*;
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let qt = resolve_type_specifiers(
        &specs_with_quals(vec![Int], vec![TypeQualifier::Const]),
        &mut table,
        &ti(),
        &mut ctx,
    )
    .unwrap();
    assert_eq!(qt.ty, int());
    assert!(qt.is_const);
    assert!(!qt.is_volatile);
    assert!(!ctx.has_errors());
}

#[test]
fn volatile_restrict_and_atomic_stack() {
    use TypeSpecifierToken::*;
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let qt = resolve_type_specifiers(
        &specs_with_quals(
            vec![Int],
            vec![
                TypeQualifier::Const,
                TypeQualifier::Volatile,
                TypeQualifier::Restrict,
                TypeQualifier::Atomic,
            ],
        ),
        &mut table,
        &ti(),
        &mut ctx,
    )
    .unwrap();
    assert!(qt.is_const && qt.is_volatile && qt.is_restrict && qt.is_atomic);
}

// ---------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------

#[test]
fn float_double_is_an_error() {
    use TypeSpecifierToken::*;
    resolve_err(vec![Float, Double]);
}

#[test]
fn unsigned_float_is_an_error() {
    use TypeSpecifierToken::*;
    resolve_err(vec![Unsigned, Float]);
}

#[test]
fn short_long_is_an_error() {
    use TypeSpecifierToken::*;
    resolve_err(vec![Short, Long]);
}

#[test]
fn long_long_long_is_an_error() {
    use TypeSpecifierToken::*;
    resolve_err(vec![Long, Long, Long]);
}

#[test]
fn signed_unsigned_is_an_error() {
    use TypeSpecifierToken::*;
    resolve_err(vec![Signed, Unsigned]);
}

#[test]
fn void_int_is_an_error() {
    use TypeSpecifierToken::*;
    resolve_err(vec![Void, Int]);
}

#[test]
fn no_specifier_at_all_is_an_error() {
    resolve_err(vec![]);
}

#[test]
fn duplicate_int_is_an_error() {
    use TypeSpecifierToken::*;
    resolve_err(vec![Int, Int]);
}

// ---------------------------------------------------------------------
// Typedef resolution
// ---------------------------------------------------------------------

#[test]
fn typedef_name_resolves_to_its_aliased_type() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table
        .declare(
            Symbol {
                id: 0,
                name: "my_int".into(),
                ty: q(int()),
                kind: SymbolKind::Typedef,
                storage: StorageClass::None,
                linkage: Linkage::None,
                span: S,
                is_defined: true,
                is_inline: false,
                is_noreturn: false,
                has_noreturn_attr: false,
            },
            &mut ctx,
        )
        .unwrap();

    let qt = resolve_type_specifiers(
        &specs(vec![TypeSpecifierToken::TypedefName("my_int".into())]),
        &mut table,
        &ti(),
        &mut ctx,
    )
    .unwrap();
    assert_eq!(qt.ty, int());
    assert!(!ctx.has_errors());
}

#[test]
fn typedef_name_combined_with_int_is_an_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    table
        .declare(
            Symbol {
                id: 0,
                name: "my_int".into(),
                ty: q(int()),
                kind: SymbolKind::Typedef,
                storage: StorageClass::None,
                linkage: Linkage::None,
                span: S,
                is_defined: true,
                is_inline: false,
                is_noreturn: false,
                has_noreturn_attr: false,
            },
            &mut ctx,
        )
        .unwrap();

    let result = resolve_type_specifiers(
        &specs(vec![
            TypeSpecifierToken::TypedefName("my_int".into()),
            TypeSpecifierToken::Int,
        ]),
        &mut table,
        &ti(),
        &mut ctx,
    );
    assert!(result.is_none());
    assert!(ctx.has_errors());
}

#[test]
fn typedef_name_without_typedef_symbol_is_an_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let result = resolve_type_specifiers(
        &specs(vec![TypeSpecifierToken::TypedefName("bogus".into())]),
        &mut table,
        &ti(),
        &mut ctx,
    );
    assert!(result.is_none());
    assert!(ctx.has_errors());
}

// ---------------------------------------------------------------------
// _Alignas on a specifier list
// ---------------------------------------------------------------------

fn int_literal(v: u64) -> Expr {
    Expr::IntLiteral {
        value: v,
        suffix: IntSuffix::None,
        span: S,
        node_id: N,
    }
}

#[test]
fn alignas_16_on_char_records_the_alignment() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let qt = resolve_type_specifiers(
        &specs_with_alignas(
            vec![TypeSpecifierToken::Char],
            AlignSpec::AlignAsExpr(Box::new(int_literal(16))),
        ),
        &mut table,
        &ti(),
        &mut ctx,
    )
    .unwrap();
    assert_eq!(qt.explicit_align, Some(16));
    assert!(!ctx.has_errors());
}

#[test]
fn alignas_double_uses_the_types_natural_alignment() {
    let tn = TypeName {
        specifiers: specs(vec![TypeSpecifierToken::Double]),
        abstract_declarator: None,
        span: S,
        node_id: N,
    };
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let qt = resolve_type_specifiers(
        &specs_with_alignas(
            vec![TypeSpecifierToken::Char],
            AlignSpec::AlignAsType(Box::new(tn)),
        ),
        &mut table,
        &ti(),
        &mut ctx,
    )
    .unwrap();
    assert_eq!(qt.explicit_align, Some(8));
    assert!(!ctx.has_errors());
}

#[test]
fn alignas_that_would_weaken_natural_alignment_errors() {
    // `_Alignas(1) int` — 1 is below int's natural 4-byte alignment.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let result = resolve_type_specifiers(
        &specs_with_alignas(
            vec![TypeSpecifierToken::Int],
            AlignSpec::AlignAsExpr(Box::new(int_literal(1))),
        ),
        &mut table,
        &ti(),
        &mut ctx,
    );
    assert!(result.is_none());
    assert!(ctx.has_errors());
}

#[test]
fn alignas_non_power_of_two_errors() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let result = resolve_type_specifiers(
        &specs_with_alignas(
            vec![TypeSpecifierToken::Int],
            AlignSpec::AlignAsExpr(Box::new(int_literal(3))),
        ),
        &mut table,
        &ti(),
        &mut ctx,
    );
    assert!(result.is_none());
    assert!(ctx.has_errors());
}

#[test]
fn alignas_zero_errors() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let result = resolve_type_specifiers(
        &specs_with_alignas(
            vec![TypeSpecifierToken::Int],
            AlignSpec::AlignAsExpr(Box::new(int_literal(0))),
        ),
        &mut table,
        &ti(),
        &mut ctx,
    );
    assert!(result.is_none());
    assert!(ctx.has_errors());
}

// ---------------------------------------------------------------------
// Struct / union / enum references
// ---------------------------------------------------------------------

#[test]
fn anonymous_struct_specifier_returns_a_fresh_struct_type() {
    use forge_parser::ast::{StructDef, StructOrUnion};
    let def = StructDef {
        kind: StructOrUnion::Struct,
        name: None,
        members: Some(Vec::new()),
        attributes: Vec::new(),
        span: S,
    };
    let qt = resolve_ok(vec![TypeSpecifierToken::Struct(def)]);
    assert!(matches!(qt.ty, Type::Struct(_)));
}

#[test]
fn struct_combined_with_int_errors() {
    use forge_parser::ast::{StructDef, StructOrUnion};
    let def = StructDef {
        kind: StructOrUnion::Struct,
        name: None,
        members: Some(Vec::new()),
        attributes: Vec::new(),
        span: S,
    };
    resolve_err(vec![
        TypeSpecifierToken::Struct(def),
        TypeSpecifierToken::Int,
    ]);
}
