//! Tests for Prompt 3.6 — GNU-extension tolerance in the parser.
//!
//! The parser is expected to *accept and ignore* every GNU extension
//! that shows up in real-world system headers:
//!
//! * `__attribute__((...))` at every declarator / declaration position
//! * `__extension__` as a no-op marker
//! * `__typeof__` / `__typeof` / `typeof` as a type specifier
//! * `__asm__` / `__asm` / `asm` labels on declarators, and
//!   `__asm__(...)` statements
//! * `__builtin_va_list`, `__builtin_offsetof`,
//!   `__builtin_types_compatible_p`, `__builtin_choose_expr`
//!
//! Every test below calls `parse_tu`, which asserts that **no error**
//! diagnostics fired.  If any of these extensions regresses to an
//! error it will be caught here rather than at the system-header
//! smoke test far down the stack.
//!
//! The AST shape these extensions produce is intentionally *lossy* —
//! the parser swallows attributes wholesale and the builtins fold to
//! placeholder literals.  We assert on declarator names and
//! translation-unit shape, not on what was thrown away.

use crate::ast::*;
use crate::decl::declarator_name;

use super::helpers::parse_tu;

// =========================================================================
// Assertion helpers
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

// =========================================================================
// §A — __attribute__ tolerance at every position
// =========================================================================

#[test]
fn attribute_before_declaration_specifiers() {
    let tu = parse_tu("__attribute__((unused)) int x;");
    let d = as_declaration(&tu.declarations[0]);
    assert_eq!(d.init_declarators.len(), 1);
}

#[test]
fn attribute_after_declaration_specifiers() {
    let tu = parse_tu("int __attribute__((unused)) x;");
    let d = as_declaration(&tu.declarations[0]);
    assert_eq!(d.init_declarators.len(), 1);
}

#[test]
fn attribute_after_declarator() {
    let tu = parse_tu("int x __attribute__((unused));");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn attribute_after_pointer_star() {
    let tu = parse_tu("int * __attribute__((aligned(16))) p;");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn attribute_on_function_declarator() {
    let tu = parse_tu("int foo(void) __attribute__((noreturn));");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn attribute_on_function_parameter() {
    let tu = parse_tu("int foo(int __attribute__((unused)) x);");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn attribute_after_array_dimensions() {
    let tu = parse_tu("int arr[10] __attribute__((aligned(32)));");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn attribute_on_struct_tag() {
    let tu = parse_tu("struct __attribute__((packed)) S { int a; int b; };");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn attribute_after_struct_member() {
    let tu = parse_tu(
        "struct S { int a __attribute__((aligned(8))); char b __attribute__((packed)); };",
    );
    as_declaration(&tu.declarations[0]);
}

#[test]
fn attribute_multi_args() {
    // glibc's `__nonnull((1, 2))` form: the balanced-paren skipper must
    // descend through nested parens.
    let tu = parse_tu("int memcpy(void *dst, const void *src, unsigned long n) __attribute__((__nonnull__((1, 2))));");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn attribute_chained_multiple_declarations() {
    // Multiple attributes in a row, as glibc often stacks them.
    let tu = parse_tu("int puts(const char *s) __attribute__((nonnull(1))) __attribute__((leaf));");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn attribute_between_init_declarators() {
    // `int x, __attribute__((unused)) y;` — attribute between commas
    // in a declarator list.
    let tu = parse_tu("int x, __attribute__((unused)) y;");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn attribute_on_typedef() {
    let tu = parse_tu("typedef int __attribute__((may_alias)) aliased_int;");
    as_declaration(&tu.declarations[0]);
}

// =========================================================================
// §B — __extension__ as a no-op marker
// =========================================================================

#[test]
fn extension_prefix_on_declaration() {
    let tu = parse_tu("__extension__ int x;");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn extension_prefix_on_function_definition() {
    let tu = parse_tu("__extension__ int f(void) { return 0; }");
    let f = as_fn_def(&tu.declarations[0]);
    assert_eq!(declarator_name(&f.declarator), Some("f"));
}

#[test]
fn extension_prefix_on_typedef() {
    let tu = parse_tu("__extension__ typedef long long llong;");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn extension_inside_function_body() {
    // glibc uses `__extension__` before declarations at any block scope.
    let tu = parse_tu("int f(void) { __extension__ int x = 0; return x; }");
    as_fn_def(&tu.declarations[0]);
}

#[test]
fn extension_inside_struct_member() {
    let tu = parse_tu("struct S { __extension__ int a; int b; };");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn extension_before_expression() {
    // `(void)(__extension__ 0)` is valid — the marker is an expression
    // prefix in this position.
    let tu = parse_tu("int f(void) { (void)(__extension__ 0); return 0; }");
    as_fn_def(&tu.declarations[0]);
}

// =========================================================================
// §C — __typeof__ / typeof as a type specifier
// =========================================================================

#[test]
fn typeof_of_identifier() {
    let tu = parse_tu("int x; __typeof__(x) y;");
    assert_eq!(tu.declarations.len(), 2);
}

#[test]
fn typeof_of_type_name() {
    let tu = parse_tu("__typeof__(int) x;");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn typeof_plain_keyword() {
    let tu = parse_tu("int a; typeof(a) b;");
    assert_eq!(tu.declarations.len(), 2);
}

#[test]
fn typeof_underscore_variant() {
    let tu = parse_tu("int a; __typeof(a) b;");
    assert_eq!(tu.declarations.len(), 2);
}

#[test]
fn typeof_of_expression() {
    let tu = parse_tu("int a; __typeof__(a + 1) b;");
    assert_eq!(tu.declarations.len(), 2);
}

// =========================================================================
// §D — __asm__ labels and __asm__ statements
// =========================================================================

#[test]
fn asm_label_on_function_declaration() {
    // glibc uses `__asm__("glibc_name")` to rename a symbol.
    let tu = parse_tu(r#"int fread(void *p, int n, int m, void *f) __asm__("__fread_chk");"#);
    as_declaration(&tu.declarations[0]);
}

#[test]
fn asm_label_on_variable_declaration() {
    let tu = parse_tu(r#"extern int errno __asm__("__errno_location");"#);
    as_declaration(&tu.declarations[0]);
}

#[test]
fn asm_statement_volatile() {
    let tu = parse_tu(r#"int f(void) { __asm__ __volatile__("nop" ::: "memory"); return 0; }"#);
    as_fn_def(&tu.declarations[0]);
}

#[test]
fn asm_statement_plain() {
    let tu = parse_tu(r#"int f(void) { __asm__("pause"); return 0; }"#);
    as_fn_def(&tu.declarations[0]);
}

#[test]
fn asm_goto_statement() {
    // `asm goto(...)` — a rarely used but perfectly legal form that
    // appears in Linux headers.
    let tu = parse_tu(
        r#"int f(void) { __asm__ goto("jmp %l0" ::: : label); return 0; label: return 1; }"#,
    );
    as_fn_def(&tu.declarations[0]);
}

// =========================================================================
// §E — __builtin_* intrinsics
// =========================================================================

#[test]
fn builtin_va_list_as_type_specifier() {
    // `__builtin_va_list` is seeded in the initial typedef scope.
    let tu = parse_tu("int f(__builtin_va_list ap) { return 0; }");
    as_fn_def(&tu.declarations[0]);
}

#[test]
fn builtin_offsetof_usage() {
    let tu = parse_tu("struct S { int a; int b; }; int x = __builtin_offsetof(struct S, b);");
    assert_eq!(tu.declarations.len(), 2);
}

#[test]
fn builtin_types_compatible_p_usage() {
    let tu = parse_tu("int x = __builtin_types_compatible_p(int, int);");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn builtin_choose_expr_usage() {
    let tu = parse_tu("int x = __builtin_choose_expr(1, 2, 3);");
    as_declaration(&tu.declarations[0]);
}

// =========================================================================
// Combined stress tests (the shapes glibc really uses)
// =========================================================================

#[test]
fn glibc_style_extern_prototype() {
    // A concrete stdio-style prototype that mixes attributes, asm
    // labels, and a leading extern — this is exactly what tripped the
    // parser before Prompt 3.6.
    let tu = parse_tu(
        r#"extern int __attribute__((nothrow, leaf)) printf(const char *__restrict __format, ...) __asm__("__printf_chk");"#,
    );
    as_declaration(&tu.declarations[0]);
}

#[test]
fn extension_wrapped_typeof_decl() {
    let tu = parse_tu("__extension__ typedef __typeof__(long) my_long;");
    as_declaration(&tu.declarations[0]);
}

#[test]
fn attribute_on_pointer_return_with_asm_label() {
    let tu = parse_tu(
        r#"__attribute__((warn_unused_result)) void * malloc(unsigned long n) __asm__("__malloc_real");"#,
    );
    as_declaration(&tu.declarations[0]);
}
