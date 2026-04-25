//! Tests for [`resolve_declarator`].
//!
//! Each test constructs a parser-level [`Declarator`] by hand (bypassing
//! the parser) and asserts on the resulting [`QualType`].  The tricky
//! cases are the spiral-rule declarators (`int (*fp)(int)` and the
//! canonical `signal` declarator) and the C17 §6.7.6.3 array-to-pointer
//! adjustment on function parameters.

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::{
    ArraySize as ParserArraySize, Declarator, DirectDeclarator, Expr, ParamDecl, PointerQualifiers,
    TypeQualifier, TypeSpecifierToken,
};
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::resolve::{resolve_declarator, resolve_type_specifiers};
use crate::scope::SymbolTable;
use crate::types::{ArraySize, ParamType, QualType, Type};

use super::helpers::{int, q, ti};

fn specs(ts: Vec<TypeSpecifierToken>) -> forge_parser::ast::DeclSpecifiers {
    forge_parser::ast::DeclSpecifiers {
        storage_class: None,
        type_specifiers: ts,
        type_qualifiers: Vec::new(),
        function_specifiers: Vec::new(),
        alignment: None,
        attributes: Vec::new(),
        span: S,
    }
}

fn specs_with_quals(
    ts: Vec<TypeSpecifierToken>,
    quals: Vec<TypeQualifier>,
) -> forge_parser::ast::DeclSpecifiers {
    forge_parser::ast::DeclSpecifiers {
        storage_class: None,
        type_specifiers: ts,
        type_qualifiers: quals,
        function_specifiers: Vec::new(),
        alignment: None,
        attributes: Vec::new(),
        span: S,
    }
}

const S: Span = Span::primary(0, 0);
const N: NodeId = NodeId::DUMMY;

fn int_literal(v: u64) -> Expr {
    Expr::IntLiteral {
        value: v,
        suffix: IntSuffix::None,
        span: S,
        node_id: N,
    }
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

fn decl_with_pointers(pointers: Vec<PointerQualifiers>, direct: DirectDeclarator) -> Declarator {
    Declarator {
        pointers,
        direct,
        span: S,
    }
}

fn pointer(quals: Vec<TypeQualifier>) -> PointerQualifiers {
    PointerQualifiers {
        qualifiers: quals,
        attributes: Vec::new(),
    }
}

fn resolve_once(base: QualType, d: Declarator) -> (Option<String>, QualType) {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let result = resolve_declarator(
        base,
        &d,
        /* param = */ false,
        &mut table,
        &ti(),
        &mut ctx,
    )
    .expect("resolve_declarator");
    assert!(
        !ctx.has_errors(),
        "unexpected diagnostics: {:?}",
        ctx.diagnostics
    );
    result
}

// ---------------------------------------------------------------------
// Plain identifier / pointer declarators
// ---------------------------------------------------------------------

#[test]
fn plain_identifier_int_x() {
    let (name, qt) = resolve_once(q(int()), decl(ident("x")));
    assert_eq!(name.as_deref(), Some("x"));
    assert_eq!(qt, q(int()));
}

#[test]
fn pointer_to_int() {
    let (name, qt) = resolve_once(
        q(int()),
        decl_with_pointers(vec![pointer(vec![])], ident("p")),
    );
    assert_eq!(name.as_deref(), Some("p"));
    match qt.ty {
        Type::Pointer { pointee } => assert_eq!(*pointee, q(int())),
        _ => panic!("expected pointer, got {:?}", qt.ty),
    }
}

#[test]
fn pointer_to_pointer_to_int() {
    let (_, qt) = resolve_once(
        q(int()),
        decl_with_pointers(vec![pointer(vec![]), pointer(vec![])], ident("pp")),
    );
    match qt.ty {
        Type::Pointer { pointee } => match pointee.ty {
            Type::Pointer { pointee: inner } => assert_eq!(*inner, q(int())),
            _ => panic!("expected **int, got inner {:?}", pointee.ty),
        },
        _ => panic!("expected pointer, got {:?}", qt.ty),
    }
}

#[test]
fn pointer_to_const_int() {
    // `const int *p` — qualifier on the pointee, not the pointer.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let base_qt = resolve_type_specifiers(
        &specs_with_quals(vec![TypeSpecifierToken::Int], vec![TypeQualifier::Const]),
        &mut table,
        &ti(),
        &mut ctx,
    )
    .unwrap();
    let (_, qt) = resolve_once(
        base_qt,
        decl_with_pointers(vec![pointer(vec![])], ident("p")),
    );
    match qt.ty {
        Type::Pointer { pointee } => {
            assert_eq!(pointee.ty, int());
            assert!(pointee.is_const);
        }
        _ => panic!("expected pointer"),
    }
}

#[test]
fn const_pointer_to_int() {
    // `int *const p` — qualifier on the pointer itself.
    let (_, qt) = resolve_once(
        q(int()),
        decl_with_pointers(vec![pointer(vec![TypeQualifier::Const])], ident("p")),
    );
    assert!(qt.is_const);
    match qt.ty {
        Type::Pointer { pointee } => assert_eq!(*pointee, q(int())),
        _ => panic!("expected pointer"),
    }
}

// ---------------------------------------------------------------------
// Array declarators
// ---------------------------------------------------------------------

#[test]
fn int_array_of_ten() {
    let arr = DirectDeclarator::Array {
        base: Box::new(ident("arr")),
        size: ParserArraySize::Expr(Box::new(int_literal(10))),
        qualifiers: Vec::new(),
        is_static: false,
        span: S,
    };
    let (name, qt) = resolve_once(q(int()), decl(arr));
    assert_eq!(name.as_deref(), Some("arr"));
    match qt.ty {
        Type::Array { element, size } => {
            assert_eq!(*element, q(int()));
            assert_eq!(size, ArraySize::Fixed(10));
        }
        _ => panic!("expected array"),
    }
}

#[test]
fn unspecified_array_is_incomplete() {
    let arr = DirectDeclarator::Array {
        base: Box::new(ident("arr")),
        size: ParserArraySize::Unspecified,
        qualifiers: Vec::new(),
        is_static: false,
        span: S,
    };
    let (_, qt) = resolve_once(q(int()), decl(arr));
    match qt.ty {
        Type::Array { size, .. } => assert_eq!(size, ArraySize::Incomplete),
        _ => panic!("expected array"),
    }
}

#[test]
fn array_of_pointers_to_int() {
    // `int *arr[10]` — array OF pointer-to-int.
    let arr = DirectDeclarator::Array {
        base: Box::new(ident("arr")),
        size: ParserArraySize::Expr(Box::new(int_literal(10))),
        qualifiers: Vec::new(),
        is_static: false,
        span: S,
    };
    let d = decl_with_pointers(vec![pointer(vec![])], arr);
    let (_, qt) = resolve_once(q(int()), d);
    match qt.ty {
        Type::Array { element, size } => {
            assert_eq!(size, ArraySize::Fixed(10));
            assert!(matches!(element.ty, Type::Pointer { .. }));
        }
        _ => panic!("expected array"),
    }
}

// ---------------------------------------------------------------------
// Function declarators
// ---------------------------------------------------------------------

#[test]
fn empty_parameter_list_is_not_a_prototype() {
    // `int f()` — classic (K&R) function declaration, not a prototype.
    let f = DirectDeclarator::Function {
        base: Box::new(ident("f")),
        params: Vec::new(),
        is_variadic: false,
        span: S,
    };
    let (_, qt) = resolve_once(q(int()), decl(f));
    match qt.ty {
        Type::Function {
            is_prototype,
            params,
            ..
        } => {
            assert!(!is_prototype, "int f() must NOT be a prototype");
            assert!(params.is_empty());
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn void_parameter_is_a_prototype_with_zero_params() {
    let void_param = ParamDecl {
        specifiers: specs(vec![TypeSpecifierToken::Void]),
        declarator: None,
        span: S,
        abstract_declarator: None,
    };
    let f = DirectDeclarator::Function {
        base: Box::new(ident("f")),
        params: vec![void_param],
        is_variadic: false,
        span: S,
    };
    let (_, qt) = resolve_once(q(int()), decl(f));
    match qt.ty {
        Type::Function {
            is_prototype,
            params,
            ..
        } => {
            assert!(is_prototype, "int f(void) IS a prototype");
            assert!(params.is_empty());
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn function_taking_two_ints() {
    let p = |name: &str| ParamDecl {
        specifiers: specs(vec![TypeSpecifierToken::Int]),
        declarator: Some(decl(ident(name))),
        span: S,
        abstract_declarator: None,
    };
    let f = DirectDeclarator::Function {
        base: Box::new(ident("add")),
        params: vec![p("a"), p("b")],
        is_variadic: false,
        span: S,
    };
    let (name, qt) = resolve_once(q(int()), decl(f));
    assert_eq!(name.as_deref(), Some("add"));
    match qt.ty {
        Type::Function {
            is_prototype,
            params,
            return_type,
            is_variadic,
        } => {
            assert!(is_prototype);
            assert!(!is_variadic);
            assert_eq!(*return_type, q(int()));
            assert_eq!(params.len(), 2);
            assert_eq!(params[0].ty, q(int()));
            assert_eq!(params[0].name.as_deref(), Some("a"));
            assert_eq!(params[1].name.as_deref(), Some("b"));
            assert!(!params[0].has_static_size);
        }
        _ => panic!("expected function"),
    }
}

// ---------------------------------------------------------------------
// Spiral rule: int (*fp)(int, int)
// ---------------------------------------------------------------------

#[test]
fn pointer_to_function_int_int_returning_int() {
    // Parser-level shape of `int (*fp)(int, int)`:
    //   Declarator { direct = Function { base = Parenthesized(*fp), ... } }
    let inner = Declarator {
        pointers: vec![pointer(vec![])],
        direct: ident("fp"),
        span: S,
    };
    let p = ParamDecl {
        specifiers: specs(vec![TypeSpecifierToken::Int]),
        declarator: None,
        span: S,
        abstract_declarator: None,
    };
    let fp_direct = DirectDeclarator::Function {
        base: Box::new(DirectDeclarator::Parenthesized(Box::new(inner))),
        params: vec![p.clone(), p],
        is_variadic: false,
        span: S,
    };
    let (name, qt) = resolve_once(q(int()), decl(fp_direct));
    assert_eq!(name.as_deref(), Some("fp"));

    // fp is pointer → function(int, int) → int.
    match qt.ty {
        Type::Pointer { pointee } => match pointee.ty {
            Type::Function {
                params,
                return_type,
                ..
            } => {
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].ty, q(int()));
                assert_eq!(*return_type, q(int()));
            }
            _ => panic!("expected function inside pointer"),
        },
        _ => panic!("expected pointer"),
    }
}

// ---------------------------------------------------------------------
// Canonical `signal` declarator:
//   void (*signal(int, void (*)(int)))(int);
//
// signal is:
//   function taking (int, pointer-to-function-taking-int-returning-void)
//   returning pointer-to-function-taking-int-returning-void
// ---------------------------------------------------------------------

#[test]
fn canonical_signal_declarator() {
    use forge_parser::ast::{AbstractDeclarator, DirectAbstractDeclarator, TypeName};

    // void (*)(int)  — abstract "pointer to function taking int".
    let handler_abs = AbstractDeclarator {
        pointers: vec![pointer(vec![])],
        direct: Some(DirectAbstractDeclarator::Function {
            base: None,
            params: vec![ParamDecl {
                specifiers: specs(vec![TypeSpecifierToken::Int]),
                declarator: None,
                span: S,
                abstract_declarator: None,
            }],
            is_variadic: false,
            span: S,
        }),
        span: S,
    };
    let _ = TypeName {
        specifiers: specs(vec![TypeSpecifierToken::Void]),
        abstract_declarator: Some(handler_abs.clone()),
        span: S,
        node_id: N,
    };

    // Inner function decl: signal(int, void (*)(int))
    // Param 1: int (anonymous)
    let p_int = ParamDecl {
        specifiers: specs(vec![TypeSpecifierToken::Int]),
        declarator: None,
        span: S,
        abstract_declarator: None,
    };
    // Param 2: void (*)(int) - abstract declarator on a param.
    // Parser-level ParamDecl uses Declarator for named params; for an
    // abstract one here we emulate by packaging the pointer-to-function
    // shape as a concrete declarator with an empty identifier? No —
    // ParamDecl.declarator is Option<Declarator>, and abstract params
    // are expressed simply by `declarator: None` with the specifiers
    // already carrying the shape.  Our parser represents `void (*)(int)`
    // as a TypeSpecifier of Void plus an abstract declarator *elsewhere*,
    // but for a parameter the idiomatic encoding is to build a full
    // Declarator that names nothing — which isn't expressible via the
    // Identifier variant.  So we lean on the TypeName-style adjustment
    // path: emit the raw function-pointer as a concrete declarator whose
    // direct is Parenthesized(Declarator { pointers=[*], direct=ident("") }).
    //
    // The parser wouldn't actually produce this shape, but for the
    // resolver we only care that the overall QualType comes out right,
    // so we construct the shape with a single anonymous inner identifier
    // and later ignore its name.
    let handler_inner = Declarator {
        pointers: vec![pointer(vec![])],
        direct: ident(""),
        span: S,
    };
    let handler_direct = DirectDeclarator::Function {
        base: Box::new(DirectDeclarator::Parenthesized(Box::new(handler_inner))),
        params: vec![ParamDecl {
            specifiers: specs(vec![TypeSpecifierToken::Int]),
            declarator: None,
            span: S,
            abstract_declarator: None,
        }],
        is_variadic: false,
        span: S,
    };
    let p_handler = ParamDecl {
        specifiers: specs(vec![TypeSpecifierToken::Void]),
        declarator: Some(decl(handler_direct)),
        span: S,
        abstract_declarator: None,
    };

    // signal(int, handler)
    let signal_call = DirectDeclarator::Function {
        base: Box::new(ident("signal")),
        params: vec![p_int, p_handler],
        is_variadic: false,
        span: S,
    };
    // (*signal(int, handler))
    let wrapped = Declarator {
        pointers: vec![pointer(vec![])],
        direct: signal_call,
        span: S,
    };
    // (*signal(...))(int) — outer function suffix
    let outer = DirectDeclarator::Function {
        base: Box::new(DirectDeclarator::Parenthesized(Box::new(wrapped))),
        params: vec![ParamDecl {
            specifiers: specs(vec![TypeSpecifierToken::Int]),
            declarator: None,
            span: S,
            abstract_declarator: None,
        }],
        is_variadic: false,
        span: S,
    };
    let d = decl(outer);

    // Base type: void.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let void_qt = resolve_type_specifiers(
        &specs(vec![TypeSpecifierToken::Void]),
        &mut table,
        &ti(),
        &mut ctx,
    )
    .unwrap();
    let (name, qt) = resolve_declarator(void_qt, &d, false, &mut table, &ti(), &mut ctx)
        .expect("resolve signal");
    assert!(
        !ctx.has_errors(),
        "signal declarator produced diagnostics: {:?}",
        ctx.diagnostics
    );
    assert_eq!(name.as_deref(), Some("signal"));

    // Expected:
    //   signal : function(int, ptr->function(int)->void)
    //            returning ptr->function(int)->void
    let (sig_params, sig_ret) = match qt.ty {
        Type::Function {
            params,
            return_type,
            is_prototype,
            ..
        } => {
            assert!(is_prototype);
            (params, return_type)
        }
        _ => panic!("signal must be a function, got {:?}", qt.ty),
    };
    assert_eq!(sig_params.len(), 2, "signal takes two parameters");
    assert_eq!(sig_params[0].ty, q(int()));
    // Parameter 1 is a pointer to a function.
    assert!(
        matches!(&sig_params[1].ty.ty, Type::Pointer { pointee }
            if matches!(pointee.ty, Type::Function { .. })),
        "second parameter must be a pointer to a function, got {:?}",
        sig_params[1].ty.ty
    );
    // Return type is a pointer to a function returning void, taking int.
    match &sig_ret.ty {
        Type::Pointer { pointee } => match &pointee.ty {
            Type::Function {
                params,
                return_type,
                ..
            } => {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].ty, q(int()));
                assert!(matches!(return_type.ty, Type::Void));
            }
            _ => panic!("return must be ptr->function"),
        },
        _ => panic!("return must be a pointer"),
    }
}

// ---------------------------------------------------------------------
// Array-parameter qualifier transfer (C17 §6.7.6.3p7)
// ---------------------------------------------------------------------

/// Build `T name[qualifiers (static)? size_expr]` as a ParamDecl.
fn array_param(
    type_tokens: Vec<TypeSpecifierToken>,
    name: &str,
    size: Option<u64>,
    quals: Vec<TypeQualifier>,
    is_static: bool,
) -> ParamDecl {
    let size = match size {
        Some(n) => ParserArraySize::Expr(Box::new(int_literal(n))),
        None => ParserArraySize::Unspecified,
    };
    ParamDecl {
        specifiers: specs(type_tokens),
        declarator: Some(decl(DirectDeclarator::Array {
            base: Box::new(ident(name)),
            size,
            qualifiers: quals,
            is_static,
            span: S,
        })),
        abstract_declarator: None,
        span: S,
    }
}

/// Resolve a function `void f(ParamDecl)` and return the single
/// parameter's ParamType.
fn resolve_single_param(pd: ParamDecl) -> ParamType {
    let f_direct = DirectDeclarator::Function {
        base: Box::new(ident("f")),
        params: vec![pd],
        is_variadic: false,
        span: S,
    };
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let (_, qt) = resolve_declarator(
        q(Type::Void),
        &decl(f_direct),
        false,
        &mut table,
        &ti(),
        &mut ctx,
    )
    .unwrap();
    assert!(
        !ctx.has_errors(),
        "unexpected diagnostics: {:?}",
        ctx.diagnostics
    );
    match qt.ty {
        Type::Function { mut params, .. } => {
            assert_eq!(params.len(), 1);
            params.remove(0)
        }
        _ => panic!("expected function"),
    }
}

#[test]
fn int_arr_const_10_yields_int_const_pointer() {
    let p = array_param(
        vec![TypeSpecifierToken::Int],
        "arr",
        Some(10),
        vec![TypeQualifier::Const],
        false,
    );
    let pt = resolve_single_param(p);
    assert!(!pt.has_static_size);
    assert!(pt.ty.is_const, "qualifier must transfer to pointer itself");
    match pt.ty.ty {
        Type::Pointer { pointee } => {
            assert!(!pointee.is_const, "pointee is plain int, not const int");
            assert_eq!(pointee.ty, int());
        }
        _ => panic!("expected pointer, got {:?}", pt.ty.ty),
    }
}

#[test]
fn char_arr_restrict_yields_char_restrict_pointer() {
    let p = array_param(
        vec![TypeSpecifierToken::Char],
        "s",
        None,
        vec![TypeQualifier::Restrict],
        false,
    );
    let pt = resolve_single_param(p);
    assert!(pt.ty.is_restrict);
    assert!(!pt.ty.is_const);
}

#[test]
fn int_arr_static_10_yields_int_pointer_with_static_flag() {
    let p = array_param(
        vec![TypeSpecifierToken::Int],
        "arr",
        Some(10),
        Vec::new(),
        true,
    );
    let pt = resolve_single_param(p);
    assert!(pt.has_static_size);
    assert!(!pt.ty.is_const);
    assert!(matches!(pt.ty.ty, Type::Pointer { .. }));
}

#[test]
fn int_arr_const_static_10_carries_both_properties() {
    let p = array_param(
        vec![TypeSpecifierToken::Int],
        "arr",
        Some(10),
        vec![TypeQualifier::Const],
        true,
    );
    let pt = resolve_single_param(p);
    assert!(pt.has_static_size);
    assert!(pt.ty.is_const);
    assert!(matches!(pt.ty.ty, Type::Pointer { .. }));
}

#[test]
fn bracket_qualifiers_on_non_parameter_arrays_are_errors() {
    // `int arr[const 10];` — only valid on a parameter.
    let arr = DirectDeclarator::Array {
        base: Box::new(ident("arr")),
        size: ParserArraySize::Expr(Box::new(int_literal(10))),
        qualifiers: vec![TypeQualifier::Const],
        is_static: false,
        span: S,
    };
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let result = resolve_declarator(
        q(int()),
        &decl(arr),
        /* is_parameter = */ false,
        &mut table,
        &ti(),
        &mut ctx,
    );
    assert!(result.is_none());
    assert!(ctx.has_errors());
}

// ---------------------------------------------------------------------
// Function-to-pointer decay for parameters
// ---------------------------------------------------------------------

#[test]
fn function_typed_parameter_decays_to_pointer() {
    // void f(int g(int)) — g has function type but is adjusted to
    // pointer-to-function.
    let inner = DirectDeclarator::Function {
        base: Box::new(ident("g")),
        params: vec![ParamDecl {
            specifiers: specs(vec![TypeSpecifierToken::Int]),
            declarator: None,
            span: S,
            abstract_declarator: None,
        }],
        is_variadic: false,
        span: S,
    };
    let p = ParamDecl {
        specifiers: specs(vec![TypeSpecifierToken::Int]),
        declarator: Some(decl(inner)),
        span: S,
        abstract_declarator: None,
    };
    let pt = resolve_single_param(p);
    assert!(matches!(pt.ty.ty, Type::Pointer { .. }));
    match pt.ty.ty {
        Type::Pointer { pointee } => assert!(matches!(pointee.ty, Type::Function { .. })),
        _ => unreachable!(),
    }
}
