//! Tests for `sizeof` (Prompt 4.4).
//!
//! `sizeof` yields a `size_t` constant for complete non-VLA types.  VLA
//! operands produce a `SizeofKind::RuntimeVla` marker so lowering knows
//! to compute the size at runtime.  Function types and incomplete types
//! are rejected.

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::{
    AbstractDeclarator, ArraySize as ParserArraySize, DeclSpecifiers, DirectAbstractDeclarator,
    Expr, TypeName, TypeSpecifierToken,
};
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::check_expr;
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{ArraySize, QualType, SizeofKind, Type};

use super::helpers::{int, q, ti};

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

fn type_name(ts: Vec<TypeSpecifierToken>) -> TypeName {
    TypeName {
        specifiers: specs(ts),
        abstract_declarator: None,
        span: S,
        node_id: N,
    }
}

fn type_name_array(ts: Vec<TypeSpecifierToken>, size_expr: Option<Expr>) -> TypeName {
    let size = match size_expr {
        Some(e) => ParserArraySize::Expr(Box::new(e)),
        None => ParserArraySize::Unspecified,
    };
    TypeName {
        specifiers: specs(ts),
        abstract_declarator: Some(AbstractDeclarator {
            pointers: Vec::new(),
            direct: Some(DirectAbstractDeclarator::Array {
                base: None,
                size,
                span: S,
            }),
            span: S,
        }),
        span: S,
        node_id: N,
    }
}

fn ident(name: &str, id: u32) -> Expr {
    Expr::Ident {
        name: name.to_string(),
        span: S,
        node_id: NodeId(id),
    }
}

fn int_lit(v: u64, id: u32) -> Expr {
    Expr::IntLiteral {
        value: v,
        suffix: IntSuffix::None,
        span: S,
        node_id: NodeId(id),
    }
}

fn sizeof_type(tn: TypeName, id: u32) -> Expr {
    Expr::SizeofType {
        type_name: Box::new(tn),
        span: S,
        node_id: NodeId(id),
    }
}

fn sizeof_expr(inner: Expr, id: u32) -> Expr {
    Expr::SizeofExpr {
        expr: Box::new(inner),
        span: S,
        node_id: NodeId(id),
    }
}

fn declare_var(table: &mut SymbolTable, name: &str, ty: QualType, ctx: &mut SemaContext) {
    let sym = Symbol {
        id: 0,
        name: name.to_string(),
        ty,
        kind: SymbolKind::Variable,
        storage: StorageClass::None,
        linkage: Linkage::None,
        span: S,
        is_defined: true,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
    };
    table.declare(sym, ctx).expect("declare must succeed");
}

#[test]
fn sizeof_int_is_four_and_yields_size_t() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let e = sizeof_type(type_name(vec![TypeSpecifierToken::Int]), 1);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Long { is_unsigned: true });
    assert_eq!(ctx.sizeof_kinds.get(&1), Some(&SizeofKind::Constant(4)));
}

#[test]
fn sizeof_expr_on_int_variable_is_four() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "x", q(int()), &mut ctx);

    let e = sizeof_expr(ident("x", 10), 11);
    let qt = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    assert_eq!(qt.ty, Type::Long { is_unsigned: true });
    assert_eq!(ctx.sizeof_kinds.get(&11), Some(&SizeofKind::Constant(4)));
}

#[test]
fn sizeof_array_uses_array_size_not_pointer_size() {
    // int arr[10]; sizeof(arr) == 40, not 8.  This proves the
    // SizeofOperand context suppresses array-to-pointer decay.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(
        &mut table,
        "arr",
        q(Type::Array {
            element: Box::new(q(int())),
            size: ArraySize::Fixed(10),
        }),
        &mut ctx,
    );

    let e = sizeof_expr(ident("arr", 20), 21);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    assert_eq!(ctx.sizeof_kinds.get(&21), Some(&SizeofKind::Constant(40)));
}

#[test]
fn sizeof_function_type_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // Declare a function `int f(void);`.
    let fn_ty = q(Type::Function {
        return_type: Box::new(q(int())),
        params: Vec::new(),
        is_variadic: false,
        is_prototype: true,
    });
    let sym = Symbol {
        id: 0,
        name: "f".to_string(),
        ty: fn_ty,
        kind: SymbolKind::Function,
        storage: StorageClass::None,
        linkage: Linkage::External,
        span: S,
        is_defined: false,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
    };
    table.declare(sym, &mut ctx).expect("declare must succeed");

    let e = sizeof_expr(ident("f", 30), 31);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("function type"));
}

#[test]
fn sizeof_vla_type_records_runtime_kind() {
    // sizeof(int[n])
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    declare_var(&mut table, "n", q(int()), &mut ctx);

    let tn = type_name_array(vec![TypeSpecifierToken::Int], Some(ident("n", 40)));
    let e = sizeof_type(tn, 41);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);

    // The VLA size expression is not an ICX, so resolve_type_name will
    // treat the dimension as variable.  sizeof then records RuntimeVla.
    match ctx.sizeof_kinds.get(&41) {
        Some(SizeofKind::RuntimeVla { .. }) => {}
        other => panic!("expected RuntimeVla kind, got {other:?}"),
    }
}

#[test]
fn sizeof_incomplete_type_is_error() {
    // `struct never_defined` — declared as a forward-reference tag only.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // Install an incomplete struct layout so the type exists but has no
    // size.  We reference it through a typedef for simplicity.
    let sid = ctx.type_ctx.fresh_struct_id();
    // default layout has is_complete = false.
    ctx.type_ctx
        .set_struct(sid, crate::types::StructLayout::default());

    declare_var(&mut table, "bad", q(Type::Struct(sid)), &mut ctx);

    let e = sizeof_expr(ident("bad", 50), 51);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
    assert!(ctx.diagnostics[0].message.contains("incomplete type"));
}

#[test]
fn sizeof_of_int_literal_is_four() {
    // sizeof(42) — an rvalue expression; the operand's type is `int`.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let e = sizeof_expr(int_lit(42, 60), 61);
    let _ = check_expr(&e, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    assert_eq!(ctx.sizeof_kinds.get(&61), Some(&SizeofKind::Constant(4)));
}

// ---------------------------------------------------------------------
// VLA edge cases — extended coverage for Prompt 4.4's RuntimeVla kind.
//
// These drive the full pipeline so the AST for `int vla[n]`,
// `int (*)[n]`, `int *[n]`, and `int[5]` operands comes from the real
// parser rather than hand-built abstract declarators.
// ---------------------------------------------------------------------

mod vla_edge_cases {
    use super::super::helpers::analyze_source;
    use crate::types::SizeofKind;

    fn has_runtime_vla(kinds: &rustc_hash::FxHashMap<u32, SizeofKind>) -> bool {
        kinds
            .values()
            .any(|k| matches!(k, SizeofKind::RuntimeVla { .. }))
    }

    fn has_constant(kinds: &rustc_hash::FxHashMap<u32, SizeofKind>, value: u64) -> bool {
        kinds
            .values()
            .any(|k| matches!(k, SizeofKind::Constant(n) if *n == value))
    }

    #[test]
    fn sizeof_vla_expression_form_records_runtime_kind() {
        let (diags, ctx, _table) = analyze_source(
            r#"
                int main(void) {
                    int n = 5;
                    int vla[n];
                    return (int)sizeof(vla);
                }
            "#,
        );
        assert!(
            !diags
                .iter()
                .any(|d| matches!(d.severity, forge_diagnostics::Severity::Error)),
            "unexpected errors: {diags:?}"
        );
        assert!(
            has_runtime_vla(&ctx.sizeof_kinds),
            "expected at least one RuntimeVla kind in {:?}",
            ctx.sizeof_kinds.values().collect::<Vec<_>>()
        );
    }

    #[test]
    fn sizeof_pointer_to_vla_is_pointer_size() {
        // C17 §6.5.3.4p2: the operand is only evaluated when its *type*
        // is a VLA type.  A pointer-to-VLA is itself a pointer type
        // (always 8 bytes on LP64), so `sizeof` is a constant.
        // Recording this as the intentional behavior — if C17
        // interpretation ever shifts, update both `type_involves_vla`
        // in expr.rs and this test.
        let (diags, ctx, _table) = analyze_source(
            r#"
                int main(void) {
                    int n = 5;
                    return (int)sizeof(int (*)[n]);
                }
            "#,
        );
        assert!(
            !diags
                .iter()
                .any(|d| matches!(d.severity, forge_diagnostics::Severity::Error)),
            "unexpected errors: {diags:?}"
        );
        assert!(
            has_constant(&ctx.sizeof_kinds, 8),
            "expected a Constant(8) sizeof on LP64, got {:?}",
            ctx.sizeof_kinds.values().collect::<Vec<_>>()
        );
    }

    #[test]
    fn sizeof_array_of_pointer_with_vla_dim_records_runtime_kind() {
        // Outer dimension is Variable even though element is a plain
        // pointer — `type_involves_vla` must still flag the outer array.
        let (diags, ctx, _table) = analyze_source(
            r#"
                int main(void) {
                    int n = 5;
                    return (int)sizeof(int *[n]);
                }
            "#,
        );
        assert!(
            !diags
                .iter()
                .any(|d| matches!(d.severity, forge_diagnostics::Severity::Error)),
            "unexpected errors: {diags:?}"
        );
        assert!(
            has_runtime_vla(&ctx.sizeof_kinds),
            "expected a RuntimeVla kind for `int *[n]`, got {:?}",
            ctx.sizeof_kinds.values().collect::<Vec<_>>()
        );
    }

    #[test]
    fn sizeof_fixed_array_is_constant_not_runtime() {
        // Regression guard: a purely constant-sized array must stay on
        // the Constant path.  20 = 5 * sizeof(int).
        let (diags, ctx, _table) = analyze_source(
            r#"
                int main(void) {
                    return (int)sizeof(int[5]);
                }
            "#,
        );
        assert!(
            !diags
                .iter()
                .any(|d| matches!(d.severity, forge_diagnostics::Severity::Error)),
            "unexpected errors: {diags:?}"
        );
        assert!(
            has_constant(&ctx.sizeof_kinds, 20),
            "expected Constant(20), got {:?}",
            ctx.sizeof_kinds.values().collect::<Vec<_>>()
        );
        assert!(
            !has_runtime_vla(&ctx.sizeof_kinds),
            "did not expect any RuntimeVla kind; got {:?}",
            ctx.sizeof_kinds.values().collect::<Vec<_>>()
        );
    }
}
