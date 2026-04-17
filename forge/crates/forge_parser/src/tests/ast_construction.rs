//! Verify AST types compile and can be manually constructed.
//!
//! These tests do not parse — they build AST nodes by hand to prove the
//! type hierarchy is well-formed and every field is reachable.

use forge_lexer::{IntSuffix, Span};

use crate::ast::*;
use crate::ast_ops::*;

/// Helper: a zero-length span for synthetic nodes.
const S: Span = Span::new(0, 0);

/// Helper: empty `DeclSpecifiers` with only `type_specifiers` filled.
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

// -----------------------------------------------------------------
// int main() { return 0; }
// -----------------------------------------------------------------

#[test]
fn function_def_int_main_return_zero() {
    let ret = Stmt::Return {
        value: Some(Box::new(Expr::IntLiteral {
            value: 0,
            suffix: IntSuffix::None,
            span: S,
        })),
        span: S,
    };

    let func = FunctionDef {
        specifiers: specs(vec![TypeSpecifierToken::Int]),
        declarator: Declarator {
            pointers: Vec::new(),
            direct: DirectDeclarator::Function {
                base: Box::new(DirectDeclarator::Identifier("main".into(), S)),
                params: Vec::new(),
                is_variadic: false,
                span: S,
            },
            span: S,
        },
        body: CompoundStmt {
            items: vec![BlockItem::Statement(ret)],
            span: S,
        },
        span: S,
    };

    let tu = TranslationUnit {
        declarations: vec![ExternalDeclaration::FunctionDef(func)],
        span: S,
    };

    assert_eq!(tu.declarations.len(), 1);
}

// -----------------------------------------------------------------
// unsigned long long x = 42;
//   → type_specifiers = vec![Unsigned, Long, Long, Int]
// -----------------------------------------------------------------

#[test]
fn declaration_unsigned_long_long_is_vec() {
    let decl = Declaration {
        specifiers: specs(vec![
            TypeSpecifierToken::Unsigned,
            TypeSpecifierToken::Long,
            TypeSpecifierToken::Long,
            TypeSpecifierToken::Int,
        ]),
        init_declarators: vec![InitDeclarator {
            declarator: Declarator {
                pointers: Vec::new(),
                direct: DirectDeclarator::Identifier("x".into(), S),
                span: S,
            },
            initializer: Some(Initializer::Expr(Box::new(Expr::IntLiteral {
                value: 42,
                suffix: IntSuffix::None,
                span: S,
            }))),
            span: S,
        }],
        span: S,
    };

    // Verify it really is a Vec with four entries.
    assert_eq!(decl.specifiers.type_specifiers.len(), 4);
}

// -----------------------------------------------------------------
// a + b * c   →   BinaryOp(+, a, BinaryOp(*, b, c))
// -----------------------------------------------------------------

#[test]
fn expr_add_mul_nesting() {
    let b_times_c = Expr::BinaryOp {
        op: BinaryOp::Mul,
        left: Box::new(Expr::Ident {
            name: "b".into(),
            span: S,
        }),
        right: Box::new(Expr::Ident {
            name: "c".into(),
            span: S,
        }),
        span: S,
    };

    let a_plus_bc = Expr::BinaryOp {
        op: BinaryOp::Add,
        left: Box::new(Expr::Ident {
            name: "a".into(),
            span: S,
        }),
        right: Box::new(b_times_c),
        span: S,
    };

    // Check the outer node is Add.
    if let Expr::BinaryOp { op, .. } = &a_plus_bc {
        assert_eq!(*op, BinaryOp::Add);
    } else {
        panic!("expected BinaryOp");
    }
}

// -----------------------------------------------------------------
// struct with bit-field member
// -----------------------------------------------------------------

#[test]
fn struct_def_with_bit_field() {
    let def = StructDef {
        kind: StructOrUnion::Struct,
        name: Some("flags".into()),
        members: Some(vec![StructMember::Field(StructField {
            specifiers: specs(vec![TypeSpecifierToken::Unsigned, TypeSpecifierToken::Int]),
            declarators: vec![StructFieldDeclarator {
                declarator: Some(Declarator {
                    pointers: Vec::new(),
                    direct: DirectDeclarator::Identifier("active".into(), S),
                    span: S,
                }),
                bit_width: Some(Box::new(Expr::IntLiteral {
                    value: 1,
                    suffix: IntSuffix::None,
                    span: S,
                })),
                span: S,
            }],
            span: S,
        })]),
        attributes: Vec::new(),
        span: S,
    };

    assert_eq!(def.kind, StructOrUnion::Struct);
    assert!(def.members.is_some());
}

// -----------------------------------------------------------------
// _Static_assert inside a struct body
// -----------------------------------------------------------------

#[test]
fn struct_member_static_assert_variant_exists() {
    let sa = StaticAssert {
        condition: Box::new(Expr::IntLiteral {
            value: 1,
            suffix: IntSuffix::None,
            span: S,
        }),
        message: Some("size check".into()),
        span: S,
    };

    let member = StructMember::StaticAssert(sa);

    // Just prove the variant exists and pattern-matches.
    assert!(matches!(member, StructMember::StaticAssert(_)));
}

// -----------------------------------------------------------------
// Stmt::Expr with span
// -----------------------------------------------------------------

#[test]
fn stmt_expr_has_span() {
    let stmt = Stmt::Expr {
        expr: Some(Box::new(Expr::Ident {
            name: "x".into(),
            span: S,
        })),
        span: Span::new(10, 12),
    };

    if let Stmt::Expr { span, .. } = &stmt {
        assert_eq!(span.start, 10);
        assert_eq!(span.end, 12);
    } else {
        panic!("expected Stmt::Expr");
    }
}
