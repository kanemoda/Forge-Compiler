//! Tests for [`check_initializer`].
//!
//! These drive the initialiser-shape rules — scalar vs brace-enclosed,
//! designator kind, excess-element warnings, and the refinement of
//! incomplete arrays from element counts or string-literal bytes.

use forge_lexer::{IntSuffix, Span, StringPrefix};
use forge_parser::ast::{
    DesignatedInit, Designator, Expr, Initializer, StructDef, StructField, StructFieldDeclarator,
    StructMember, StructOrUnion,
};
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::declare::check_initializer;
use crate::scope::SymbolTable;
use crate::types::{ArraySize, QualType, Signedness, Type};

use super::helpers::{int, q, ti};

const S: Span = Span::new(0, 0);
const N: NodeId = NodeId::DUMMY;

// ---------------------------------------------------------------------
// Construction helpers
// ---------------------------------------------------------------------

fn int_lit(v: u64) -> Expr {
    Expr::IntLiteral {
        value: v,
        suffix: IntSuffix::None,
        span: S,
        node_id: N,
    }
}

fn string_lit(s: &str) -> Expr {
    Expr::StringLiteral {
        value: s.to_string(),
        prefix: StringPrefix::None,
        span: S,
        node_id: N,
    }
}

fn expr_init(v: u64) -> Initializer {
    Initializer::Expr(Box::new(int_lit(v)))
}

fn brace_init(items: Vec<DesignatedInit>) -> Initializer {
    Initializer::List {
        items,
        span: S,
        node_id: N,
    }
}

fn item_plain(init: Initializer) -> DesignatedInit {
    DesignatedInit {
        designators: Vec::new(),
        initializer: Box::new(init),
        span: S,
    }
}

fn item_index(index: u64, init: Initializer) -> DesignatedInit {
    DesignatedInit {
        designators: vec![Designator::Index(Box::new(int_lit(index)))],
        initializer: Box::new(init),
        span: S,
    }
}

fn item_field(field: &str, init: Initializer) -> DesignatedInit {
    DesignatedInit {
        designators: vec![Designator::Field(field.to_string())],
        initializer: Box::new(init),
        span: S,
    }
}

fn array_of(elem: Type, size: ArraySize) -> QualType {
    QualType::unqualified(Type::Array {
        element: Box::new(QualType::unqualified(elem)),
        size,
    })
}

fn char_array(size: ArraySize) -> QualType {
    array_of(
        Type::Char {
            signedness: Signedness::Plain,
        },
        size,
    )
}

// ---------------------------------------------------------------------
// Scalar initialisation
// ---------------------------------------------------------------------

#[test]
fn scalar_init_with_single_expr_is_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = q(int());
    let out = check_initializer(&expr_init(42), &t, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    assert_eq!(out, t);
}

#[test]
fn scalar_init_with_braced_single_expr_is_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = q(int());
    let init = brace_init(vec![item_plain(expr_init(42))]);
    check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn scalar_init_with_excess_braced_elements_warns() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = q(int());
    let init = brace_init(vec![item_plain(expr_init(1)), item_plain(expr_init(2))]);
    check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.diagnostics.iter().any(|d| d.message.contains("excess")),
        "expected an excess-elements diagnostic, got {:?}",
        ctx.diagnostics
    );
}

#[test]
fn scalar_init_with_designator_errors() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = q(int());
    let init = brace_init(vec![item_field("x", expr_init(1))]);
    check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.has_errors(),
        "expected a diagnostic for designator with scalar"
    );
}

// ---------------------------------------------------------------------
// Array initialisation
// ---------------------------------------------------------------------

#[test]
fn fixed_array_init_with_fewer_elements_is_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = array_of(int(), ArraySize::Fixed(3));
    let init = brace_init(vec![item_plain(expr_init(1)), item_plain(expr_init(2))]);
    check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn fixed_array_init_with_excess_elements_warns() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = array_of(int(), ArraySize::Fixed(2));
    let init = brace_init(vec![
        item_plain(expr_init(1)),
        item_plain(expr_init(2)),
        item_plain(expr_init(3)),
    ]);
    check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.diagnostics.iter().any(|d| d.message.contains("excess")),
        "expected excess-elements warning: {:?}",
        ctx.diagnostics
    );
}

#[test]
fn incomplete_array_is_refined_by_element_count() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = array_of(int(), ArraySize::Incomplete);
    let init = brace_init(vec![
        item_plain(expr_init(1)),
        item_plain(expr_init(2)),
        item_plain(expr_init(3)),
    ]);
    let refined = check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let Type::Array { size, .. } = refined.ty else {
        panic!("expected refined array type");
    };
    assert_eq!(size, ArraySize::Fixed(3));
}

#[test]
fn incomplete_array_with_index_designator_is_refined_to_highest_plus_one() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = array_of(int(), ArraySize::Incomplete);
    let init = brace_init(vec![item_index(5, expr_init(42)), item_plain(expr_init(1))]);
    let refined = check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let Type::Array { size, .. } = refined.ty else {
        panic!("expected refined array type");
    };
    // After `[5] = 42`, the next plain item lands at index 6.
    assert_eq!(size, ArraySize::Fixed(7));
}

#[test]
fn array_init_with_field_designator_errors() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = array_of(int(), ArraySize::Fixed(3));
    let init = brace_init(vec![item_field("x", expr_init(1))]);
    check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.has_errors(),
        "expected a diagnostic for field designator on array"
    );
}

#[test]
fn scalar_target_with_array_init_errors() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    // `int x = { ... array-only init ... }` with an excess element is
    // warned, but we also want to confirm that *arrays* cannot take a
    // bare scalar init.
    let t = array_of(int(), ArraySize::Fixed(3));
    let init = expr_init(42);
    check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.has_errors(),
        "expected diagnostic for scalar init to array"
    );
}

// ---------------------------------------------------------------------
// Char array from string literal
// ---------------------------------------------------------------------

#[test]
fn char_array_init_from_string_refines_incomplete_size() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = char_array(ArraySize::Incomplete);
    let init = Initializer::Expr(Box::new(string_lit("abc")));
    let refined = check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let Type::Array { size, .. } = refined.ty else {
        panic!("expected refined array type");
    };
    // "abc" has 3 bytes + trailing NUL → 4.
    assert_eq!(size, ArraySize::Fixed(4));
}

#[test]
fn char_array_init_from_string_preserves_fixed_size() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = char_array(ArraySize::Fixed(10));
    let init = Initializer::Expr(Box::new(string_lit("hi")));
    let out = check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
    assert_eq!(out, t);
}

// ---------------------------------------------------------------------
// Struct initialisation (shape only; full type checks in Phase 4.4)
// ---------------------------------------------------------------------

fn register_struct_x_y(ctx: &mut SemaContext, table: &mut SymbolTable) -> QualType {
    use forge_parser::ast::{DeclSpecifiers as PDS, Declarator as PDecl, DirectDeclarator as PDir};
    let specs = PDS {
        storage_class: None,
        type_specifiers: vec![forge_parser::ast::TypeSpecifierToken::Int],
        type_qualifiers: Vec::new(),
        function_specifiers: Vec::new(),
        alignment: None,
        attributes: Vec::new(),
        span: S,
    };
    let decl_x = PDecl {
        pointers: Vec::new(),
        direct: PDir::Identifier("x".into(), S),
        span: S,
    };
    let decl_y = PDecl {
        pointers: Vec::new(),
        direct: PDir::Identifier("y".into(), S),
        span: S,
    };
    let members = vec![
        StructMember::Field(StructField {
            specifiers: specs.clone(),
            declarators: vec![StructFieldDeclarator {
                declarator: Some(decl_x),
                bit_width: None,
                span: S,
            }],
            span: S,
            node_id: N,
        }),
        StructMember::Field(StructField {
            specifiers: specs,
            declarators: vec![StructFieldDeclarator {
                declarator: Some(decl_y),
                bit_width: None,
                span: S,
            }],
            span: S,
            node_id: N,
        }),
    ];
    let def = StructDef {
        kind: StructOrUnion::Struct,
        name: Some("Point".into()),
        members: Some(members),
        attributes: Vec::new(),
        span: S,
    };
    let sid = ctx.type_ctx.fresh_struct_id();
    ctx.type_ctx
        .set_struct(sid, crate::types::StructLayout::default());
    crate::layout::complete_struct(sid, &def, table, &ti(), ctx);
    q(Type::Struct(sid))
}

#[test]
fn struct_brace_init_accepts_positional_elements() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = register_struct_x_y(&mut ctx, &mut table);
    let init = brace_init(vec![item_plain(expr_init(1)), item_plain(expr_init(2))]);
    check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);
}

#[test]
fn struct_brace_init_excess_elements_warns() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = register_struct_x_y(&mut ctx, &mut table);
    let init = brace_init(vec![
        item_plain(expr_init(1)),
        item_plain(expr_init(2)),
        item_plain(expr_init(3)),
    ]);
    check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.diagnostics.iter().any(|d| d.message.contains("excess")),
        "expected excess-elements warning: {:?}",
        ctx.diagnostics
    );
}

#[test]
fn struct_brace_init_with_array_designator_errors() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = register_struct_x_y(&mut ctx, &mut table);
    let init = brace_init(vec![item_index(0, expr_init(1))]);
    check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
}

#[test]
fn struct_bare_scalar_init_errors() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = register_struct_x_y(&mut ctx, &mut table);
    let init = expr_init(1);
    check_initializer(&init, &t, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
}

// ---------------------------------------------------------------------
// Void target rejection
// ---------------------------------------------------------------------

#[test]
fn void_target_rejects_any_initializer() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let t = q(Type::Void);
    check_initializer(&expr_init(0), &t, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors());
}

// ---------------------------------------------------------------------
// Scalar initialiser RHS type checking (source-level regressions)
//
// These drive the full lexer → parser → sema pipeline so they can
// exercise name resolution, constant evaluation, and assignability
// inside an initialiser's RHS.  Prior to this patch the RHS of a
// scalar initialiser was silently accepted no matter its type.
// ---------------------------------------------------------------------

mod scalar_rhs_type_check {
    use super::super::helpers::{assert_source_clean, assert_source_has_errors};

    // -------- error cases --------

    #[test]
    fn initializer_string_to_int_is_error() {
        assert_source_has_errors(
            r#"
                int main(void) {
                    int x = "hello";
                    return x;
                }
            "#,
        );
    }

    #[test]
    fn initializer_double_to_pointer_is_error() {
        assert_source_has_errors(
            r#"
                int main(void) {
                    int *p = 3.14;
                    return p == 0;
                }
            "#,
        );
    }

    #[test]
    fn initializer_struct_to_int_is_error() {
        assert_source_has_errors(
            r#"
                struct S { int a; };
                int main(void) {
                    struct S s = { 0 };
                    int x = s;
                    return x;
                }
            "#,
        );
    }

    #[test]
    fn initializer_undeclared_identifier_is_caught() {
        // REGRESSION GUARD for 4.7.2 review: prior to the fix the RHS
        // never reached `check_expr`, so an undeclared identifier on the
        // RHS of a scalar initialiser was silently accepted.
        assert_source_has_errors(
            r#"
                int main(void) {
                    int x = unknown_name;
                    return x;
                }
            "#,
        );
    }

    #[test]
    fn initializer_qualifier_mismatch() {
        assert_source_has_errors(
            r#"
                int main(void) {
                    const int *src = 0;
                    int *dst = src;
                    return dst == 0;
                }
            "#,
        );
    }

    // -------- happy-path cases --------

    #[test]
    fn initializer_int_literal_to_int_is_ok() {
        assert_source_clean(
            r#"
                int main(void) {
                    int x = 42;
                    return x;
                }
            "#,
        );
    }

    #[test]
    fn initializer_int_to_long_applies_conversion() {
        // The `42` → `long` widening must succeed and must record an
        // `ArithmeticConversion` on the RHS so later lowering knows to
        // sign-extend.  If the wiring regresses, implicit_convs stays
        // empty and the assertion fires.
        use super::super::helpers::analyze_source;
        use crate::types::ImplicitConversion;

        let (diags, ctx, _table) = analyze_source(
            r#"
                int main(void) {
                    long y = 42;
                    return (int)y;
                }
            "#,
        );
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| matches!(d.severity, forge_diagnostics::Severity::Error))
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        let has_arith_conv = ctx
            .implicit_convs
            .values()
            .any(|c| matches!(c, ImplicitConversion::ArithmeticConversion { .. }));
        assert!(
            has_arith_conv,
            "expected ArithmeticConversion in implicit_convs, got {:?}",
            ctx.implicit_convs.values().collect::<Vec<_>>()
        );
    }

    #[test]
    fn initializer_float_to_int_converts_without_error() {
        // GCC warns on the narrowing — we accept silently for now.  The
        // important property is that it does NOT produce an error.
        assert_source_clean(
            r#"
                int main(void) {
                    int x = 3.14;
                    return x;
                }
            "#,
        );
    }

    #[test]
    fn initializer_null_constant_to_pointer_is_ok() {
        use super::super::helpers::analyze_source;
        use crate::types::ImplicitConversion;

        let (diags, ctx, _table) = analyze_source(
            r#"
                int main(void) {
                    int *p = 0;
                    return p == 0;
                }
            "#,
        );
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| matches!(d.severity, forge_diagnostics::Severity::Error))
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        let has_null_conv = ctx
            .implicit_convs
            .values()
            .any(|c| matches!(c, ImplicitConversion::NullPointerConversion));
        assert!(
            has_null_conv,
            "expected NullPointerConversion in implicit_convs, got {:?}",
            ctx.implicit_convs.values().collect::<Vec<_>>()
        );
    }

    #[test]
    fn initializer_string_to_char_array_is_ok() {
        assert_source_clean(
            r#"
                int main(void) {
                    char s[] = "hello";
                    return s[0];
                }
            "#,
        );
    }

    // -------- brace-list leaf propagation --------

    #[test]
    fn initializer_brace_list_leaf_is_type_checked() {
        // REGRESSION GUARD: confirms the scalar fix propagates through
        // recursive `check_initializer` calls for brace lists.
        assert_source_has_errors(
            r#"
                int main(void) {
                    int arr[3] = { 1, "hello", 3 };
                    return arr[0];
                }
            "#,
        );
    }
}
