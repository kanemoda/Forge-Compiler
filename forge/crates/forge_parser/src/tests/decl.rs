//! Tests for Prompt 3.3: declaration-specifier parsing, declarators,
//! parameter lists, full `parse_type_name()`, and declaration-vs-expression
//! disambiguation.

use crate::ast::*;
use crate::ast_ops::BinaryOp;
use crate::decl::declarator_name;
use crate::parser::Parser;

use super::helpers::{lex, parse_decl, parse_decls, parse_type_name};

// =========================================================================
// AST matchers
// =========================================================================

/// Extract the identifier of an `InitDeclarator`, panicking if it has no
/// name (which shouldn't happen in any of the well-formed test inputs).
fn init_decl_name(id: &InitDeclarator) -> &str {
    declarator_name(&id.declarator).expect("declarator without an identifier")
}

/// The single init-declarator of a declaration, asserting exactly one.
fn sole_init_decl(d: &Declaration) -> &InitDeclarator {
    assert_eq!(
        d.init_declarators.len(),
        1,
        "expected exactly one init-declarator, got {}",
        d.init_declarators.len()
    );
    &d.init_declarators[0]
}

/// Assert the expression is an `IntLiteral` with the given value.
fn expect_int_lit(e: &Expr, want: u64) {
    match e {
        Expr::IntLiteral { value, .. } => assert_eq!(*value, want),
        other => panic!("expected IntLiteral({want}), got {other:?}"),
    }
}

/// Unwrap a `BinaryOp`, asserting its operator.
fn expect_binop(e: &Expr, want: BinaryOp) -> (&Expr, &Expr) {
    match e {
        Expr::BinaryOp {
            op, left, right, ..
        } if *op == want => (left, right),
        other => panic!("expected BinaryOp({want:?}), got {other:?}"),
    }
}

/// Unwrap `Initializer::Expr`.
fn expr_init(init: &Initializer) -> &Expr {
    match init {
        Initializer::Expr(e) => e,
        Initializer::List { .. } => panic!("expected Initializer::Expr, got List"),
    }
}

/// Walk into the identifier at the bottom of a direct-declarator.  Skips
/// over array/function suffixes and parenthesised wrappers.
fn direct_ident(d: &DirectDeclarator) -> &str {
    match d {
        DirectDeclarator::Identifier(name, _) => name,
        DirectDeclarator::Parenthesized(inner) => direct_ident(&inner.direct),
        DirectDeclarator::Array { base, .. } | DirectDeclarator::Function { base, .. } => {
            direct_ident(base)
        }
    }
}

/// True if the storage class is `Typedef`.
fn is_typedef_decl(d: &Declaration) -> bool {
    matches!(d.specifiers.storage_class, Some(StorageClass::Typedef))
}

// =========================================================================
// Simple primitive types
// =========================================================================

#[test]
fn int_x() {
    let d = parse_decl("int x;");
    assert!(matches!(
        d.specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::Int]
    ));
    let id = sole_init_decl(&d);
    assert_eq!(init_decl_name(id), "x");
    assert!(id.initializer.is_none());
    assert!(id.declarator.pointers.is_empty());
}

#[test]
fn unsigned_long_long_x() {
    let d = parse_decl("unsigned long long x;");
    assert!(matches!(
        d.specifiers.type_specifiers.as_slice(),
        [
            TypeSpecifierToken::Unsigned,
            TypeSpecifierToken::Long,
            TypeSpecifierToken::Long
        ]
    ));
    assert_eq!(init_decl_name(sole_init_decl(&d)), "x");
}

#[test]
fn long_unsigned_int_long_preserves_order() {
    // Order is preserved; sema validates the (legal) combination.
    let d = parse_decl("long unsigned int long x;");
    assert!(matches!(
        d.specifiers.type_specifiers.as_slice(),
        [
            TypeSpecifierToken::Long,
            TypeSpecifierToken::Unsigned,
            TypeSpecifierToken::Int,
            TypeSpecifierToken::Long
        ]
    ));
}

// =========================================================================
// Pointer prefixes and qualifiers
// =========================================================================

#[test]
fn const_int_ptr() {
    // `const int *p;` — const qualifier on outer specifiers, then one
    // unqualified pointer level.
    let d = parse_decl("const int *p;");
    assert_eq!(d.specifiers.type_qualifiers, vec![TypeQualifier::Const]);
    assert!(matches!(
        d.specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::Int]
    ));
    let id = sole_init_decl(&d);
    assert_eq!(init_decl_name(id), "p");
    assert_eq!(id.declarator.pointers.len(), 1);
    assert!(id.declarator.pointers[0].qualifiers.is_empty());
}

#[test]
fn ptr_const() {
    // `int *const p;` — pointer to int, with const on the pointer itself.
    let d = parse_decl("int *const p;");
    let id = sole_init_decl(&d);
    assert_eq!(id.declarator.pointers.len(), 1);
    assert_eq!(
        id.declarator.pointers[0].qualifiers,
        vec![TypeQualifier::Const]
    );
}

#[test]
fn three_pointer_levels() {
    // `int **const *volatile p;` — three `*`s:
    //   *         (no quals)
    //   * const
    //   * volatile
    let d = parse_decl("int **const *volatile p;");
    let id = sole_init_decl(&d);
    let ptrs = &id.declarator.pointers;
    assert_eq!(ptrs.len(), 3);
    assert!(ptrs[0].qualifiers.is_empty());
    assert_eq!(ptrs[1].qualifiers, vec![TypeQualifier::Const]);
    assert_eq!(ptrs[2].qualifiers, vec![TypeQualifier::Volatile]);
    assert_eq!(init_decl_name(id), "p");
}

// =========================================================================
// Initializers
// =========================================================================

#[test]
fn init_int_literal() {
    let d = parse_decl("int x = 5;");
    let id = sole_init_decl(&d);
    let init = id.initializer.as_ref().expect("expected initializer");
    expect_int_lit(expr_init(init), 5);
}

#[test]
fn init_binary_expr() {
    let d = parse_decl("int x = 1 + 2;");
    let id = sole_init_decl(&d);
    let init = id.initializer.as_ref().expect("expected initializer");
    let (l, r) = expect_binop(expr_init(init), BinaryOp::Add);
    expect_int_lit(l, 1);
    expect_int_lit(r, 2);
}

// =========================================================================
// Multiple init-declarators
// =========================================================================

#[test]
fn multiple_init_declarators() {
    let d = parse_decl("int x, y, *z;");
    assert_eq!(d.init_declarators.len(), 3);
    assert_eq!(init_decl_name(&d.init_declarators[0]), "x");
    assert_eq!(init_decl_name(&d.init_declarators[1]), "y");
    assert_eq!(init_decl_name(&d.init_declarators[2]), "z");
    // Only the third declarator has a pointer.
    assert!(d.init_declarators[0].declarator.pointers.is_empty());
    assert!(d.init_declarators[1].declarator.pointers.is_empty());
    assert_eq!(d.init_declarators[2].declarator.pointers.len(), 1);
}

// =========================================================================
// Typedef tracking
// =========================================================================

#[test]
fn typedef_registers_name() {
    let d = parse_decl("typedef int MyInt;");
    assert!(is_typedef_decl(&d));
    assert_eq!(init_decl_name(sole_init_decl(&d)), "MyInt");
}

#[test]
fn typedef_then_use_as_type() {
    // Two declarations in sequence: the second should see the typedef.
    let decls = parse_decls("typedef int MyInt; MyInt x;");
    assert_eq!(decls.len(), 2);

    assert!(is_typedef_decl(&decls[0]));
    assert_eq!(init_decl_name(sole_init_decl(&decls[0])), "MyInt");

    let d2 = &decls[1];
    assert!(matches!(
        d2.specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::TypedefName(name)] if name == "MyInt"
    ));
    assert_eq!(init_decl_name(sole_init_decl(d2)), "x");
}

#[test]
fn typedef_disambiguates_pointer_vs_multiplication() {
    // `typedef int T; T * x;` — `T * x` is a pointer declaration, NOT the
    // multiplication expression it would be if T weren't a typedef.
    let decls = parse_decls("typedef int T; T * x;");
    assert_eq!(decls.len(), 2);

    let d2 = &decls[1];
    assert!(matches!(
        d2.specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::TypedefName(name)] if name == "T"
    ));
    let id = sole_init_decl(d2);
    assert_eq!(init_decl_name(id), "x");
    assert_eq!(id.declarator.pointers.len(), 1);
}

#[test]
fn non_typedef_identifier_does_not_start_declaration() {
    // `int x;` then `x * y;` — after the first declaration ends, the
    // tokens starting with `x` should NOT register as a declaration.
    let tokens = lex("int x; x * y;");
    let mut parser = Parser::new(tokens);
    let _first = parser.parse_declaration();
    // Now the cursor is at `x` of `x * y;`.  `x` is not a typedef, so
    // this must NOT be a declaration start.
    assert!(
        !parser.is_start_of_declaration(),
        "`x` is not a typedef; `x * y;` must be parsed as a statement"
    );
}

#[test]
fn typedef_shadowed_by_local_declarator() {
    // `typedef int T; T T;` — the second T is a declarator name, NOT a
    // typedef reference, because a type specifier (the first T) has
    // already been seen in this declaration.
    let decls = parse_decls("typedef int T; T T;");
    assert_eq!(decls.len(), 2);

    let d2 = &decls[1];
    // First T is the type specifier.
    assert!(matches!(
        d2.specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::TypedefName(name)] if name == "T"
    ));
    // Second T is the declarator name.
    assert_eq!(init_decl_name(sole_init_decl(d2)), "T");
}

// =========================================================================
// Parenthesised declarators
// =========================================================================

#[test]
fn parenthesized_declarator() {
    // `int (*fp);` — the `(*fp)` is a parenthesised declarator wrapping a
    // pointer declarator for `fp`.
    let d = parse_decl("int (*fp);");
    let id = sole_init_decl(&d);
    match &id.declarator.direct {
        DirectDeclarator::Parenthesized(inner) => {
            assert_eq!(inner.pointers.len(), 1);
            match &inner.direct {
                DirectDeclarator::Identifier(name, _) => assert_eq!(name, "fp"),
                other => panic!("expected Identifier, got {other:?}"),
            }
        }
        other => panic!("expected Parenthesized, got {other:?}"),
    }
}

// =========================================================================
// Function declarators
// =========================================================================

#[test]
fn function_two_named_params() {
    let d = parse_decl("int f(int a, char *b);");
    let id = sole_init_decl(&d);
    let DirectDeclarator::Function {
        params,
        is_variadic,
        ..
    } = &id.declarator.direct
    else {
        panic!(
            "expected function declarator, got {:?}",
            id.declarator.direct
        );
    };
    assert!(!is_variadic);
    assert_eq!(params.len(), 2);

    // param 0: int a
    assert!(matches!(
        params[0].specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::Int]
    ));
    let decl0 = params[0].declarator.as_ref().expect("param 0 concrete");
    assert_eq!(direct_ident(&decl0.direct), "a");
    assert!(decl0.pointers.is_empty());

    // param 1: char *b
    assert!(matches!(
        params[1].specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::Char]
    ));
    let decl1 = params[1].declarator.as_ref().expect("param 1 concrete");
    assert_eq!(direct_ident(&decl1.direct), "b");
    assert_eq!(decl1.pointers.len(), 1);

    // Function name itself
    assert_eq!(direct_ident(&id.declarator.direct), "f");
}

#[test]
fn function_void_params() {
    // `int f(void);` — explicitly no parameters.
    let d = parse_decl("int f(void);");
    let id = sole_init_decl(&d);
    let DirectDeclarator::Function {
        params,
        is_variadic,
        ..
    } = &id.declarator.direct
    else {
        panic!("expected function declarator");
    };
    assert!(!is_variadic);
    assert!(
        params.is_empty(),
        "(void) must yield zero ParamDecls, got {params:?}"
    );
}

#[test]
fn function_variadic_abstract_first_param() {
    // `int f(int, ...);` — first param is abstract (no name), variadic.
    let d = parse_decl("int f(int, ...);");
    let id = sole_init_decl(&d);
    let DirectDeclarator::Function {
        params,
        is_variadic,
        ..
    } = &id.declarator.direct
    else {
        panic!("expected function declarator");
    };
    assert!(is_variadic);
    assert_eq!(params.len(), 1);
    assert!(
        params[0].declarator.is_none(),
        "abstract param should have declarator=None"
    );
    assert!(matches!(
        params[0].specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::Int]
    ));
}

// =========================================================================
// Anonymous parameters with shape (regression for the Phase 4 fix-up)
// =========================================================================
//
// Until Phase 4 fix-up, `parse_param_decl` parsed the abstract declarator
// of an unnamed parameter and then *discarded* it, so `void g(int *);`
// silently degraded to `void g(int);`.  These tests pin down the
// post-fix shape so the bug can't sneak back in.

/// Convenience: extract the function declarator's parameter list.
fn function_params(d: &Declaration) -> &[ParamDecl] {
    let id = sole_init_decl(d);
    let DirectDeclarator::Function { params, .. } = &id.declarator.direct else {
        panic!("expected function declarator");
    };
    params
}

#[test]
fn anonymous_pointer_parameter() {
    let d = parse_decl("void g(int *);");
    let params = function_params(&d);
    assert_eq!(params.len(), 1);
    assert!(params[0].declarator.is_none());
    let abs = params[0]
        .abstract_declarator
        .as_ref()
        .expect("anonymous param must carry an abstract declarator");
    assert_eq!(abs.pointers.len(), 1);
    assert!(abs.pointers[0].qualifiers.is_empty());
    assert!(abs.direct.is_none());
}

#[test]
fn anonymous_pointer_to_pointer_parameter() {
    let d = parse_decl("void g(int **);");
    let params = function_params(&d);
    assert_eq!(params.len(), 1);
    let abs = params[0]
        .abstract_declarator
        .as_ref()
        .expect("anonymous param must carry an abstract declarator");
    assert_eq!(abs.pointers.len(), 2, "two pointer levels");
    assert!(abs.direct.is_none());
}

#[test]
fn anonymous_pointer_to_const_int_parameter() {
    // `const` here qualifies the pointee, not the pointer; it lives on
    // the parameter's specifiers, not on the abstract declarator.
    let d = parse_decl("void g(const int *);");
    let params = function_params(&d);
    assert!(params[0]
        .specifiers
        .type_qualifiers
        .iter()
        .any(|q| matches!(q, TypeQualifier::Const)));
    let abs = params[0]
        .abstract_declarator
        .as_ref()
        .expect("anonymous param must carry an abstract declarator");
    assert_eq!(abs.pointers.len(), 1);
    assert!(
        abs.pointers[0].qualifiers.is_empty(),
        "const belongs on the pointee, not the pointer prefix"
    );
}

#[test]
fn multiple_anonymous_pointer_parameters() {
    let d = parse_decl("void g(int *, char *, double *);");
    let params = function_params(&d);
    assert_eq!(params.len(), 3);
    for (i, p) in params.iter().enumerate() {
        assert!(p.declarator.is_none(), "param {i} should be anonymous");
        let abs = p
            .abstract_declarator
            .as_ref()
            .unwrap_or_else(|| panic!("param {i} missing abstract declarator"));
        assert_eq!(abs.pointers.len(), 1, "param {i} pointer level");
        assert!(
            abs.direct.is_none(),
            "param {i} has no array/function suffix"
        );
    }
    assert!(matches!(
        params[0].specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::Int]
    ));
    assert!(matches!(
        params[1].specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::Char]
    ));
    assert!(matches!(
        params[2].specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::Double]
    ));
}

#[test]
fn mixed_anonymous_and_named_pointer_parameters() {
    let d = parse_decl("void g(int *p, char *, double *q);");
    let params = function_params(&d);
    assert_eq!(params.len(), 3);

    // p: named pointer
    let dp = params[0].declarator.as_ref().expect("param 0 named");
    assert_eq!(direct_ident(&dp.direct), "p");
    assert_eq!(dp.pointers.len(), 1);
    assert!(params[0].abstract_declarator.is_none());

    // anonymous middle
    assert!(params[1].declarator.is_none());
    let mid = params[1]
        .abstract_declarator
        .as_ref()
        .expect("middle param abstract");
    assert_eq!(mid.pointers.len(), 1);

    // q: named pointer
    let dq = params[2].declarator.as_ref().expect("param 2 named");
    assert_eq!(direct_ident(&dq.direct), "q");
    assert_eq!(dq.pointers.len(), 1);
    assert!(params[2].abstract_declarator.is_none());
}

#[test]
fn anonymous_pointer_with_qualifier_after_star() {
    // `int * const` — the pointer itself is const-qualified (the
    // pointee is plain int).  The qualifier rides on the pointer-prefix
    // entry, not on the parameter's specifiers.
    let d = parse_decl("void g(int * const);");
    let params = function_params(&d);
    let abs = params[0]
        .abstract_declarator
        .as_ref()
        .expect("anonymous param must carry an abstract declarator");
    assert_eq!(abs.pointers.len(), 1);
    assert!(
        abs.pointers[0]
            .qualifiers
            .iter()
            .any(|q| matches!(q, TypeQualifier::Const)),
        "const after `*` belongs to the pointer prefix"
    );
    // And the parameter's outer specifiers must NOT carry that const.
    assert!(
        !params[0]
            .specifiers
            .type_qualifiers
            .iter()
            .any(|q| matches!(q, TypeQualifier::Const)),
        "outer const would mean the pointee, not the pointer"
    );
}

#[test]
fn anonymous_pointer_to_function_parameter() {
    // `int (*)(int)` — pointer to function taking int returning int.
    // After the fix this should round-trip through the abstract path
    // without losing either the `*` or the inner `(int)` suffix.
    let d = parse_decl("void g(int (*)(int));");
    let params = function_params(&d);
    assert_eq!(params.len(), 1);
    let abs = params[0]
        .abstract_declarator
        .as_ref()
        .expect("anonymous param must carry an abstract declarator");
    // Outer pointer prefix is empty; the `*` is inside the parens.
    assert!(abs.pointers.is_empty());
    let direct = abs
        .direct
        .as_ref()
        .expect("function pointer has a direct part");
    let DirectAbstractDeclarator::Function { base, params, .. } = direct else {
        panic!("expected function suffix at the outermost direct part, got {direct:?}");
    };
    // Inner parens hold the `(*)` — pointer-to-something.
    let inner_base = base
        .as_ref()
        .expect("function-pointer abstract has a parenthesised base");
    let DirectAbstractDeclarator::Parenthesized(inner) = inner_base.as_ref() else {
        panic!("expected `(*)` parenthesised abstract, got {inner_base:?}");
    };
    assert_eq!(inner.pointers.len(), 1, "the inner `*`");
    assert!(inner.direct.is_none());
    // The function suffix takes one `int` parameter.
    assert_eq!(params.len(), 1);
    assert!(matches!(
        params[0].specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::Int]
    ));
}

// =========================================================================
// Array declarators
// =========================================================================

#[test]
fn array_with_size() {
    let d = parse_decl("int arr[10];");
    let id = sole_init_decl(&d);
    let DirectDeclarator::Array { base, size, .. } = &id.declarator.direct else {
        panic!("expected array declarator");
    };
    // base is the identifier `arr`
    match base.as_ref() {
        DirectDeclarator::Identifier(name, _) => assert_eq!(name, "arr"),
        other => panic!("expected identifier base, got {other:?}"),
    }
    match size {
        ArraySize::Expr(e) => expect_int_lit(e, 10),
        other => panic!("expected ArraySize::Expr, got {other:?}"),
    }
}

#[test]
fn array_unspecified_size() {
    let d = parse_decl("int arr[];");
    let id = sole_init_decl(&d);
    let DirectDeclarator::Array { size, .. } = &id.declarator.direct else {
        panic!("expected array declarator");
    };
    assert!(matches!(size, ArraySize::Unspecified));
}

// =========================================================================
// Combined: pointer-to-function, array-of-function-pointer
// =========================================================================

#[test]
fn pointer_to_function() {
    // `int (*fp)(int, int);`
    //   fp : pointer to function(int, int) returning int
    let d = parse_decl("int (*fp)(int, int);");
    let id = sole_init_decl(&d);

    // Outermost direct is Function(base = Parenthesized(*fp), params=...)
    let DirectDeclarator::Function {
        base,
        params,
        is_variadic,
        ..
    } = &id.declarator.direct
    else {
        panic!("expected outer Function, got {:?}", id.declarator.direct);
    };
    assert!(!is_variadic);
    assert_eq!(params.len(), 2);
    for p in params {
        assert!(p.declarator.is_none(), "abstract parameter");
        assert!(matches!(
            p.specifiers.type_specifiers.as_slice(),
            [TypeSpecifierToken::Int]
        ));
    }

    // Inside base: Parenthesized(Declarator { pointers=[*], direct=fp })
    let DirectDeclarator::Parenthesized(inner) = base.as_ref() else {
        panic!("expected Parenthesized base, got {base:?}");
    };
    assert_eq!(inner.pointers.len(), 1);
    match &inner.direct {
        DirectDeclarator::Identifier(name, _) => assert_eq!(name, "fp"),
        other => panic!("expected Identifier inside parens, got {other:?}"),
    }
}

#[test]
fn array_of_function_pointers() {
    // `int (*arr[10])(void);`
    //   arr : array[10] of (pointer to function(void) returning int)
    let d = parse_decl("int (*arr[10])(void);");
    let id = sole_init_decl(&d);

    // Outermost: Function(base = Parenthesized(...), params=[])
    let DirectDeclarator::Function {
        base,
        params,
        is_variadic,
        ..
    } = &id.declarator.direct
    else {
        panic!("expected outer Function");
    };
    assert!(!is_variadic);
    assert!(params.is_empty(), "`(void)` should yield no params");

    // Inside parens: Declarator { pointers=[*], direct=Array(base=arr, size=10) }
    let DirectDeclarator::Parenthesized(inner) = base.as_ref() else {
        panic!("expected Parenthesized");
    };
    assert_eq!(inner.pointers.len(), 1);
    let DirectDeclarator::Array {
        base: arr_base,
        size,
        ..
    } = &inner.direct
    else {
        panic!("expected Array inside parens, got {:?}", inner.direct);
    };
    match arr_base.as_ref() {
        DirectDeclarator::Identifier(name, _) => assert_eq!(name, "arr"),
        other => panic!("expected Identifier `arr`, got {other:?}"),
    }
    match size {
        ArraySize::Expr(e) => expect_int_lit(e, 10),
        other => panic!("expected size=10, got {other:?}"),
    }
}

// =========================================================================
// parse_type_name — smoke tests
// =========================================================================

#[test]
fn type_name_int() {
    let tn = parse_type_name("int").expect("should parse");
    assert!(matches!(
        tn.specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::Int]
    ));
    assert!(tn.abstract_declarator.is_none());
}

#[test]
fn type_name_const_char_ptr() {
    let tn = parse_type_name("const char *").expect("should parse");
    assert_eq!(tn.specifiers.type_qualifiers, vec![TypeQualifier::Const]);
    assert!(matches!(
        tn.specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::Char]
    ));
    let ad = tn
        .abstract_declarator
        .as_ref()
        .expect("expected abstract declarator");
    assert_eq!(ad.pointers.len(), 1);
    assert!(ad.direct.is_none());
}

#[test]
fn type_name_rejects_non_type_start() {
    // A plain identifier that's not a typedef → not a type-name.
    assert!(parse_type_name("foo").is_none());
}
