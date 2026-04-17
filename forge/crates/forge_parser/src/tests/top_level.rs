//! Tests for Prompt 3.5 — translation-unit and external-declaration
//! parsing (function definitions vs plain declarations vs file-scope
//! `_Static_assert`).

use crate::ast::*;
use crate::decl::declarator_name;

use super::helpers::parse_tu;

// =========================================================================
// Helpers
// =========================================================================

fn as_fn_def(decl: &ExternalDeclaration) -> &FunctionDef {
    match decl {
        ExternalDeclaration::FunctionDef(f) => f,
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

fn as_declaration(decl: &ExternalDeclaration) -> &Declaration {
    match decl {
        ExternalDeclaration::Declaration(d) => d,
        other => panic!("expected Declaration, got {other:?}"),
    }
}

fn as_static_assert(decl: &ExternalDeclaration) -> &StaticAssert {
    match decl {
        ExternalDeclaration::StaticAssert(s) => s,
        other => panic!("expected StaticAssert, got {other:?}"),
    }
}

// =========================================================================
// Function definitions
// =========================================================================

#[test]
fn int_main_return_zero() {
    let tu = parse_tu("int main(void) { return 0; }");
    assert_eq!(tu.declarations.len(), 1);
    let f = as_fn_def(&tu.declarations[0]);
    assert_eq!(declarator_name(&f.declarator), Some("main"));
    assert_eq!(f.body.items.len(), 1);
    match &f.body.items[0] {
        BlockItem::Statement(Stmt::Return { .. }) => {}
        other => panic!("expected return stmt, got {other:?}"),
    }
}

#[test]
fn function_with_parameters() {
    let tu = parse_tu("int add(int a, int b) { return a + b; }");
    let f = as_fn_def(&tu.declarations[0]);
    let DirectDeclarator::Function { params, .. } = &f.declarator.direct else {
        panic!("expected function declarator");
    };
    assert_eq!(params.len(), 2);
}

#[test]
fn function_with_pointer_return() {
    let tu = parse_tu("int *get(void) { return 0; }");
    let f = as_fn_def(&tu.declarations[0]);
    assert_eq!(f.declarator.pointers.len(), 1);
    assert_eq!(declarator_name(&f.declarator), Some("get"));
}

#[test]
fn multiple_function_defs() {
    let src = "\
        int inc(int x) { return x + 1; } \
        int dec(int x) { return x - 1; } \
    ";
    let tu = parse_tu(src);
    assert_eq!(tu.declarations.len(), 2);
    let f0 = as_fn_def(&tu.declarations[0]);
    let f1 = as_fn_def(&tu.declarations[1]);
    assert_eq!(declarator_name(&f0.declarator), Some("inc"));
    assert_eq!(declarator_name(&f1.declarator), Some("dec"));
}

#[test]
fn function_body_exercises_statement_dispatcher() {
    // One function whose body touches every major statement kind.
    let src = "int f(int n) { \
        int sum = 0; \
        if (n < 0) return -1; else sum = 0; \
        for (int i = 0; i < n; i = i + 1) { \
            if (i == 3) continue; \
            if (i == 7) break; \
            sum = sum + i; \
        } \
        switch (sum) { case 0: return 0; default: return sum; } \
        while (sum) sum = sum - 1; \
        do { sum = sum + 1; } while (sum < n); \
        end: return sum; \
    }";
    let tu = parse_tu(src);
    assert_eq!(tu.declarations.len(), 1);
    let f = as_fn_def(&tu.declarations[0]);
    // At the very least, we expect many block items.
    assert!(f.body.items.len() >= 7);
}

// =========================================================================
// Declaration vs function-definition disambiguation
// =========================================================================

#[test]
fn function_declaration_vs_definition() {
    // First is a *declaration* (prototype, ends in `;`), second is a
    // *definition* (has a compound body).
    let tu = parse_tu("int f(int);\nint f(int x) { return x; }");
    assert_eq!(tu.declarations.len(), 2);
    let d0 = as_declaration(&tu.declarations[0]);
    assert_eq!(d0.init_declarators.len(), 1);
    assert!(d0.init_declarators[0].initializer.is_none());
    let _ = as_fn_def(&tu.declarations[1]);
}

#[test]
fn multiple_init_declarators_at_file_scope() {
    let tu = parse_tu("int a, b = 2, c;");
    assert_eq!(tu.declarations.len(), 1);
    let d = as_declaration(&tu.declarations[0]);
    assert_eq!(d.init_declarators.len(), 3);
    assert!(d.init_declarators[0].initializer.is_none());
    assert!(d.init_declarators[1].initializer.is_some());
    assert!(d.init_declarators[2].initializer.is_none());
}

#[test]
fn typedef_then_use() {
    // Defining a typedef at file scope must register the name so
    // subsequent declarations see it.
    let tu = parse_tu("typedef int MyInt; MyInt x = 3;");
    assert_eq!(tu.declarations.len(), 2);
    let d1 = as_declaration(&tu.declarations[1]);
    // `MyInt` becomes a TypedefName specifier.
    assert!(matches!(
        d1.specifiers.type_specifiers.as_slice(),
        [TypeSpecifierToken::TypedefName(n)] if n == "MyInt"
    ));
}

// =========================================================================
// Struct/union/enum-only declarations at file scope
// =========================================================================

#[test]
fn struct_definition_with_no_declarators() {
    let tu = parse_tu("struct Point { int x; int y; };");
    let d = as_declaration(&tu.declarations[0]);
    assert!(d.init_declarators.is_empty());
}

#[test]
fn enum_definition_with_no_declarators() {
    let tu = parse_tu("enum Color { RED, GREEN, BLUE };");
    let d = as_declaration(&tu.declarations[0]);
    assert!(d.init_declarators.is_empty());
}

// =========================================================================
// _Static_assert at file scope
// =========================================================================

#[test]
fn file_scope_static_assert_with_message() {
    let tu = parse_tu("_Static_assert(sizeof(int) >= 4, \"need 32-bit int\");");
    assert_eq!(tu.declarations.len(), 1);
    let sa = as_static_assert(&tu.declarations[0]);
    assert_eq!(sa.message.as_deref(), Some("need 32-bit int"));
}

#[test]
fn file_scope_static_assert_without_message() {
    let tu = parse_tu("_Static_assert(1);");
    let sa = as_static_assert(&tu.declarations[0]);
    assert!(sa.message.is_none());
}

// =========================================================================
// Stray semicolons at file scope
// =========================================================================

#[test]
fn stray_semicolons_ignored() {
    let tu = parse_tu(";; int x; ;;;int y;;");
    // Only the two real declarations survive.
    assert_eq!(tu.declarations.len(), 2);
    let d0 = as_declaration(&tu.declarations[0]);
    let d1 = as_declaration(&tu.declarations[1]);
    assert_eq!(
        declarator_name(&d0.init_declarators[0].declarator),
        Some("x")
    );
    assert_eq!(
        declarator_name(&d1.init_declarators[0].declarator),
        Some("y")
    );
}

#[test]
fn empty_translation_unit() {
    let tu = parse_tu("");
    assert!(tu.declarations.is_empty());
}
