//! Stress / edge-case tests for the parser.
//!
//! These confirm the parser does not panic, overflow the stack, or
//! produce spurious diagnostics when given deeply nested, very wide, or
//! otherwise pathological syntactic input.  Every test asserts that
//! parsing succeeds with zero error-severity diagnostics.

use super::helpers::*;
use crate::ast::{
    BlockItem, DirectDeclarator, Expr, ExternalDeclaration, Initializer, Stmt, StructMember,
    TypeSpecifierToken,
};
use crate::ast_ops::BinaryOp;

// -----------------------------------------------------------------------
// 1. Empty translation unit
// -----------------------------------------------------------------------

#[test]
fn empty_file_parses_to_empty_translation_unit() {
    let tu = parse_tu("");
    assert!(tu.declarations.is_empty());
}

// -----------------------------------------------------------------------
// 2. 50 nested compound statements
// -----------------------------------------------------------------------

#[test]
fn fifty_nested_blocks_do_not_overflow() {
    let depth = 50;
    let mut src = String::from("void f(void) ");
    for _ in 0..depth {
        src.push('{');
    }
    for _ in 0..depth {
        src.push('}');
    }

    let tu = parse_tu(&src);
    let ExternalDeclaration::FunctionDef(fd) = &tu.declarations[0] else {
        panic!("expected function def");
    };

    // Walk `depth - 1` layers of single-item compound blocks inside the
    // outer function body.  Each layer is exactly `BlockItem::Statement`
    // wrapping a `Stmt::Compound` that contains the next layer.
    let mut current = &fd.body;
    for _ in 0..(depth - 1) {
        assert_eq!(current.items.len(), 1);
        let BlockItem::Statement(Stmt::Compound(inner)) = &current.items[0] else {
            panic!("expected nested compound statement");
        };
        current = inner;
    }
    assert!(current.items.is_empty(), "innermost block must be empty");
}

// -----------------------------------------------------------------------
// 3. Long additive expression
// -----------------------------------------------------------------------

#[test]
fn additive_expression_with_two_hundred_terms() {
    let n = 200;
    let mut src = String::from("1");
    for i in 2..=n {
        src.push_str(&format!(" + {i}"));
    }

    // The Pratt parser is left-associative for `+`, so the resulting
    // tree has exactly `n - 1` binary-op nodes along its left spine.
    let mut e = parse_expr(&src);
    let mut depth = 0usize;
    while let Expr::BinaryOp { op, left, .. } = e {
        assert!(matches!(op, BinaryOp::Add));
        depth += 1;
        e = *left;
    }
    assert_eq!(depth, n - 1);
    assert!(matches!(e, Expr::IntLiteral { .. }));
}

// -----------------------------------------------------------------------
// 4. Struct with 100 members
// -----------------------------------------------------------------------

#[test]
fn struct_with_one_hundred_members() {
    let n = 100;
    let mut src = String::from("struct Big {\n");
    for i in 0..n {
        src.push_str(&format!("    int m{i};\n"));
    }
    src.push_str("};\n");

    let decls = parse_decls(&src);
    assert_eq!(decls.len(), 1);
    let Some(TypeSpecifierToken::Struct(def)) = decls[0].specifiers.type_specifiers.first() else {
        panic!("expected struct specifier");
    };
    let members = def.members.as_ref().expect("struct must have body");
    let field_count: usize = members
        .iter()
        .map(|m| match m {
            StructMember::Field(f) => f.declarators.len(),
            StructMember::StaticAssert(_) => 0,
        })
        .sum();
    assert_eq!(field_count, n);
}

// -----------------------------------------------------------------------
// 5. Function with 50 parameters
// -----------------------------------------------------------------------

#[test]
fn function_declaration_with_fifty_parameters() {
    let n = 50;
    let mut src = String::from("void f(");
    for i in 0..n {
        if i > 0 {
            src.push_str(", ");
        }
        src.push_str(&format!("int p{i}"));
    }
    src.push_str(");");

    let decls = parse_decls(&src);
    assert_eq!(decls.len(), 1);
}

// -----------------------------------------------------------------------
// 6. 20 levels of pointer declarators
// -----------------------------------------------------------------------

#[test]
fn pointer_declarator_twenty_levels_deep() {
    let n = 20;
    let mut src = String::from("int ");
    for _ in 0..n {
        src.push('*');
    }
    src.push_str("p;");

    let decls = parse_decls(&src);
    let init_decls = &decls[0].init_declarators;
    assert_eq!(init_decls.len(), 1);

    // All 20 stars live in the declarator's `pointers` Vec (one entry per
    // `*`, each with its own qualifiers), and the direct-declarator is a
    // plain identifier.
    assert_eq!(init_decls[0].declarator.pointers.len(), n);
    assert!(matches!(
        init_decls[0].declarator.direct,
        DirectDeclarator::Identifier(_, _)
    ));
}

// -----------------------------------------------------------------------
// 7. Deeply nested initializer lists
// -----------------------------------------------------------------------

#[test]
fn nested_initializer_ten_levels_deep() {
    let depth = 10;
    let mut dims = String::new();
    for _ in 0..depth {
        dims.push_str("[1]");
    }
    let mut init = String::from("1");
    for _ in 0..depth {
        init = format!("{{ {init} }}");
    }
    let src = format!("int a{dims} = {init};");

    let decls = parse_decls(&src);
    let init_expr = decls[0].init_declarators[0]
        .initializer
        .as_ref()
        .expect("must have initializer");

    // Walk the `Initializer::List` chain and confirm it is exactly `depth`
    // levels before bottoming out in an `Initializer::Expr`.
    let mut current = init_expr;
    for _ in 0..depth {
        let Initializer::List { items, .. } = current else {
            panic!("expected nested list");
        };
        assert_eq!(items.len(), 1);
        current = &items[0].initializer;
    }
    assert!(matches!(current, Initializer::Expr(_)));
}

// -----------------------------------------------------------------------
// 8. Long chain of call expressions
// -----------------------------------------------------------------------

#[test]
fn one_hundred_chained_calls() {
    // `f(0)(1)(2)...(99)` — each postfix `(..)` wraps the previous
    // expression in a `FunctionCall` node.
    let n = 100;
    let mut src = String::from("f");
    for i in 0..n {
        src.push_str(&format!("({i})"));
    }

    let mut e = parse_expr(&src);
    let mut call_depth = 0usize;
    while let Expr::FunctionCall { callee, .. } = e {
        call_depth += 1;
        e = *callee;
    }
    assert_eq!(call_depth, n);
    assert!(matches!(e, Expr::Ident { .. }));
}

// -----------------------------------------------------------------------
// 9. Typedef shadowed by a local variable of the same name
// -----------------------------------------------------------------------

#[test]
fn typedef_shadowed_by_local_declaration() {
    // `T` is a typedef at file scope.  Inside `f`, `int T = 42;` uses
    // `T` as the declarator name — the parser's `seen_type_specifier`
    // flag must break out of the specifier loop once `int` is seen, so
    // the following typedef-named identifier is consumed as a
    // declarator rather than as another type specifier.  After the
    // function body, `T outside;` still uses `T` as a typedef at file
    // scope.  (Full scope-based shadow tracking — where `T` after the
    // inner declaration refers to the variable — is a semantic-analysis
    // concern and is not yet modelled by the parser.)
    let src = r"
        typedef int T;
        void f(void) {
            int T = 42;
        }
        T outside;
    ";

    let tu = parse_tu(src);
    assert_eq!(tu.declarations.len(), 3);
}

// -----------------------------------------------------------------------
// 10. Empty function body
// -----------------------------------------------------------------------

#[test]
fn empty_function_body_is_a_compound_with_zero_items() {
    let tu = parse_tu("void f(void) {}");
    let ExternalDeclaration::FunctionDef(fd) = &tu.declarations[0] else {
        panic!("expected function definition");
    };
    assert!(fd.body.items.is_empty());
}

// -----------------------------------------------------------------------
// 11. 20 init-declarators in one declaration
// -----------------------------------------------------------------------

#[test]
fn twenty_init_declarators_in_a_single_declaration() {
    let n = 20;
    let mut src = String::from("int ");
    for i in 0..n {
        if i > 0 {
            src.push_str(", ");
        }
        src.push_str(&format!("a{i} = {i}"));
    }
    src.push(';');

    let decls = parse_decls(&src);
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].init_declarators.len(), n);
}

// -----------------------------------------------------------------------
// 12. Complex declarator: function returning pointer to array of
//     function pointers.
// -----------------------------------------------------------------------

#[test]
fn complex_declarator_function_returning_pointer_to_array_of_fnptrs() {
    // `int (*(*f(int x))[10])(double);`
    //   - `f` is a function taking `int x`
    //   - returning a pointer to
    //   - an array[10] of
    //   - pointers to
    //   - functions taking `double` returning `int`
    let src = "int (*(*f(int x))[10])(double);";
    let decls = parse_decls(src);
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].init_declarators.len(), 1);
}

// -----------------------------------------------------------------------
// 13. Every compound assignment operator in one function
// -----------------------------------------------------------------------

#[test]
fn all_assignment_operators_parse() {
    let src = r"
        void f(int a, int b) {
            a = b;
            a += b;
            a -= b;
            a *= b;
            a /= b;
            a %= b;
            a <<= b;
            a >>= b;
            a &= b;
            a |= b;
            a ^= b;
        }
    ";

    let tu = parse_tu(src);
    let ExternalDeclaration::FunctionDef(fd) = &tu.declarations[0] else {
        panic!("expected function");
    };
    // 11 expression-statement items, one per assignment.
    assert_eq!(fd.body.items.len(), 11);
}

// -----------------------------------------------------------------------
// 14. _Generic with ten type associations
// -----------------------------------------------------------------------

#[test]
fn generic_selection_with_ten_associations() {
    let src = "_Generic(x, \
            char: 1, \
            short: 2, \
            int: 3, \
            long: 4, \
            long long: 5, \
            float: 6, \
            double: 7, \
            long double: 8, \
            unsigned int: 9, \
            signed int: 10, \
            default: 0)";

    let expr = parse_expr(src);
    let Expr::GenericSelection { associations, .. } = expr else {
        panic!("expected Expr::GenericSelection");
    };
    // 10 typed associations + 1 default = 11 total.
    assert_eq!(associations.len(), 11);
    assert!(associations.iter().any(|a| a.type_name.is_none()));
}

// -----------------------------------------------------------------------
// 15. Deeply nested parenthesised expression
// -----------------------------------------------------------------------

#[test]
fn deeply_parenthesised_expression_parses() {
    // 100 open-parens, an integer, 100 close-parens — the Pratt parser
    // must recurse cleanly into each nesting level.
    let n = 100;
    let mut src = String::new();
    for _ in 0..n {
        src.push('(');
    }
    src.push('1');
    for _ in 0..n {
        src.push(')');
    }

    // `(expr)` is transparent — the parser returns the inner expression
    // directly, so 100 levels of parens still produce just an
    // `IntLiteral`.
    let e = parse_expr(&src);
    assert!(matches!(e, Expr::IntLiteral { .. }));
}
