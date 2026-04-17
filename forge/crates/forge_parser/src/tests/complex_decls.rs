//! Full-pipeline integration tests: tricky real-world C declarations
//! that exercise declarator nesting, parameter lists, and pointers all
//! at once.  Originally requested as part of Prompt 3.4.

use crate::ast::*;
use crate::decl::declarator_name;

use super::helpers::parse_decl;

fn sole_decl(d: &Declaration) -> &Declarator {
    assert_eq!(d.init_declarators.len(), 1);
    &d.init_declarators[0].declarator
}

fn unwrap_paren(d: &DirectDeclarator) -> &Declarator {
    match d {
        DirectDeclarator::Parenthesized(inner) => inner,
        other => panic!("expected Parenthesized, got {other:?}"),
    }
}

#[test]
fn pointer_to_function_returning_pointer_to_array() {
    // `int (*(*fp)(int))[10];`
    //   fp : pointer to function (int) returning pointer to array[10] of int
    let d = parse_decl("int (*(*fp)(int))[10];");
    let decl = sole_decl(&d);
    assert_eq!(declarator_name(decl), Some("fp"));

    // Outer direct: Array[10] wrapping a Parenthesized(*…)
    let DirectDeclarator::Array { base, size, .. } = &decl.direct else {
        panic!("expected outer Array, got {:?}", decl.direct);
    };
    match size {
        ArraySize::Expr(e) => match e.as_ref() {
            Expr::IntLiteral { value, .. } => assert_eq!(*value, 10),
            other => panic!("expected [10], got {other:?}"),
        },
        other => panic!("expected ArraySize::Expr, got {other:?}"),
    }

    // base = Parenthesized( Declarator { pointers=[*], direct=... } )
    let inner1 = unwrap_paren(base);
    assert_eq!(inner1.pointers.len(), 1);

    // inside: Function(base=Parenthesized(*fp), params=[int])
    let DirectDeclarator::Function {
        base: fn_base,
        params,
        is_variadic,
        ..
    } = &inner1.direct
    else {
        panic!("expected Function, got {:?}", inner1.direct);
    };
    assert!(!is_variadic);
    assert_eq!(params.len(), 1);
    assert!(params[0].declarator.is_none());

    let inner2 = unwrap_paren(fn_base);
    assert_eq!(inner2.pointers.len(), 1);
    match &inner2.direct {
        DirectDeclarator::Identifier(name, _) => assert_eq!(name, "fp"),
        other => panic!("expected Identifier `fp`, got {other:?}"),
    }
}

#[test]
fn signal_function_signature() {
    // `void (*signal(int sig, void (*func)(int)))(int);`
    //   signal : function(int sig, void (*)(int)) returning
    //            pointer to function(int) returning void
    let d = parse_decl("void (*signal(int sig, void (*func)(int)))(int);");
    let decl = sole_decl(&d);
    assert_eq!(declarator_name(decl), Some("signal"));

    // Outermost: Function (args =[int]) — the *returned* function.
    let DirectDeclarator::Function {
        base,
        params,
        is_variadic,
        ..
    } = &decl.direct
    else {
        panic!("expected outer Function, got {:?}", decl.direct);
    };
    assert!(!is_variadic);
    assert_eq!(params.len(), 1);
    assert!(params[0].declarator.is_none());

    // Inside: Parenthesized( Declarator { pointers=[*], direct=Function(signal, (int sig, void (*func)(int))) } )
    let inner = unwrap_paren(base);
    assert_eq!(inner.pointers.len(), 1);

    let DirectDeclarator::Function {
        base: sig_base,
        params: sig_params,
        ..
    } = &inner.direct
    else {
        panic!("expected inner Function, got {:?}", inner.direct);
    };
    match sig_base.as_ref() {
        DirectDeclarator::Identifier(name, _) => assert_eq!(name, "signal"),
        other => panic!("expected signal identifier, got {other:?}"),
    }

    assert_eq!(sig_params.len(), 2);

    // param[0]: int sig
    assert_eq!(
        declarator_name(sig_params[0].declarator.as_ref().expect("concrete")),
        Some("sig")
    );

    // param[1]: void (*func)(int)
    let func_decl = sig_params[1].declarator.as_ref().expect("concrete");
    assert_eq!(declarator_name(func_decl), Some("func"));
    // Outer direct is Function(base=Parenthesized(*func), params=[int])
    let DirectDeclarator::Function {
        base: fn_base,
        params: fn_params,
        ..
    } = &func_decl.direct
    else {
        panic!("expected Function, got {:?}", func_decl.direct);
    };
    assert_eq!(fn_params.len(), 1);
    let inner2 = unwrap_paren(fn_base);
    assert_eq!(inner2.pointers.len(), 1);
    match &inner2.direct {
        DirectDeclarator::Identifier(name, _) => assert_eq!(name, "func"),
        other => panic!("expected `func`, got {other:?}"),
    }
}

#[test]
fn array_of_function_pointers_named() {
    // `int (*fps[10])(void);`
    let d = parse_decl("int (*fps[10])(void);");
    let decl = sole_decl(&d);
    assert_eq!(declarator_name(decl), Some("fps"));

    // Outer: Function(base = Parenthesized(...), params=[])
    let DirectDeclarator::Function { base, params, .. } = &decl.direct else {
        panic!("expected outer Function");
    };
    assert!(params.is_empty(), "(void) yields zero params");

    let inner = unwrap_paren(base);
    assert_eq!(inner.pointers.len(), 1);
    let DirectDeclarator::Array {
        base: arr_base,
        size,
        ..
    } = &inner.direct
    else {
        panic!("expected Array inside parens, got {:?}", inner.direct);
    };
    match size {
        ArraySize::Expr(e) => match e.as_ref() {
            Expr::IntLiteral { value, .. } => assert_eq!(*value, 10),
            other => panic!("expected [10], got {other:?}"),
        },
        other => panic!("expected sized array, got {other:?}"),
    }
    match arr_base.as_ref() {
        DirectDeclarator::Identifier(name, _) => assert_eq!(name, "fps"),
        other => panic!("expected `fps`, got {other:?}"),
    }
}
