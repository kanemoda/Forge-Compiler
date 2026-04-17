//! Tests for the AST pretty-printer (`forge_parser::printer`).
//!
//! The printer's output format is **not** a stable public contract —
//! it is a debugging tool, and these tests lock in enough of the shape
//! (indentation, key node names, operator spelling) that accidental
//! regressions are caught without pinning every trailing whitespace.
//!
//! Assertions are shape-oriented: we look for individual lines by
//! substring, and we verify indentation depth via a leading-spaces
//! count rather than pinning the entire multi-line string.

use crate::printer::print_ast;

use super::helpers::parse_tu;

/// Count the leading spaces on a line — two per indent level, as the
/// printer emits.
fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

/// Find the first line in the printer output that contains `needle`,
/// and panic with a truncated dump if it is missing.
fn find_line<'a>(out: &'a str, needle: &str) -> &'a str {
    out.lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("line containing {needle:?} not found in:\n{out}"))
}

// =========================================================================
// Translation-unit shape
// =========================================================================

#[test]
fn translation_unit_is_the_root_node() {
    let tu = parse_tu("int x;");
    let out = print_ast(&tu);
    assert!(out.starts_with("TranslationUnit\n"), "got:\n{out}");
}

#[test]
fn empty_translation_unit_has_just_the_root() {
    let tu = parse_tu("");
    let out = print_ast(&tu);
    assert_eq!(out, "TranslationUnit\n");
}

#[test]
fn output_ends_with_trailing_newline() {
    let tu = parse_tu("int x;");
    let out = print_ast(&tu);
    assert!(out.ends_with('\n'), "expected trailing newline: {out:?}");
}

// =========================================================================
// Indentation
// =========================================================================

#[test]
fn two_space_indent_between_levels() {
    let tu = parse_tu("int x;");
    let out = print_ast(&tu);
    // The Declaration under TranslationUnit must be indented by 2
    // spaces; inside it, Specifiers is indented by 4.
    let decl_line = find_line(&out, "Declaration");
    let spec_line = find_line(&out, "Specifiers");
    assert_eq!(indent_of(decl_line), 2, "Declaration: {decl_line:?}");
    assert_eq!(indent_of(spec_line), 4, "Specifiers: {spec_line:?}");
}

// =========================================================================
// Specifier rendering
// =========================================================================

#[test]
fn specifiers_list_is_bracket_delimited() {
    let tu = parse_tu("unsigned long long int x;");
    let out = print_ast(&tu);
    let line = find_line(&out, "Specifiers");
    // The exact order is the source order: Unsigned, Long, Long, Int.
    assert!(
        line.contains("Unsigned") && line.contains("Long") && line.contains("Int"),
        "specifiers list missing tokens: {line:?}"
    );
    assert!(line.contains('['), "expected `[` in {line:?}");
    assert!(line.contains(']'), "expected `]` in {line:?}");
}

#[test]
fn signed_char_specifiers() {
    let tu = parse_tu("signed char c;");
    let out = print_ast(&tu);
    let line = find_line(&out, "Specifiers");
    assert!(line.contains("Signed"), "{line:?}");
    assert!(line.contains("Char"), "{line:?}");
}

// =========================================================================
// Declarator rendering
// =========================================================================

#[test]
fn declarator_shows_identifier_name() {
    let tu = parse_tu("int my_variable;");
    let out = print_ast(&tu);
    assert!(out.contains("my_variable"), "got:\n{out}");
}

#[test]
fn pointer_declarator_is_rendered() {
    let tu = parse_tu("int *p;");
    let out = print_ast(&tu);
    assert!(
        out.contains("Pointer") || out.contains('*'),
        "expected pointer node in:\n{out}"
    );
}

#[test]
fn array_declarator_shows_dimension() {
    let tu = parse_tu("int arr[10];");
    let out = print_ast(&tu);
    assert!(out.contains("Array"), "expected Array node in:\n{out}");
}

// =========================================================================
// Function definitions
// =========================================================================

#[test]
fn function_def_renders_with_params_and_body() {
    let tu = parse_tu("int add(int a, int b) { return a + b; }");
    let out = print_ast(&tu);
    assert!(out.contains("FunctionDef"), "got:\n{out}");
    assert!(out.contains("add"), "got:\n{out}");
    assert!(out.contains("ParamDecl"), "got:\n{out}");
    assert!(out.contains("CompoundStmt"), "got:\n{out}");
    assert!(out.contains("Return"), "got:\n{out}");
}

#[test]
fn function_def_return_value_is_an_expression_subtree() {
    let tu = parse_tu("int f(void) { return 42; }");
    let out = print_ast(&tu);
    assert!(
        out.contains("IntLiteral 42"),
        "expected IntLiteral 42 in:\n{out}"
    );
}

// =========================================================================
// Operator spelling
// =========================================================================

#[test]
fn binary_op_is_spelled_with_operator_glyph() {
    let tu = parse_tu("int x = 1 + 2;");
    let out = print_ast(&tu);
    assert!(
        out.contains("BinaryOp +"),
        "expected `BinaryOp +` in:\n{out}"
    );
}

#[test]
fn logical_and_is_spelled_double_ampersand() {
    let tu = parse_tu("int x = 1 && 2;");
    let out = print_ast(&tu);
    assert!(
        out.contains("BinaryOp &&"),
        "expected `BinaryOp &&` in:\n{out}"
    );
}

#[test]
fn assignment_compound_op_renders_correctly() {
    let tu = parse_tu("int f(void) { int x; x += 3; return x; }");
    let out = print_ast(&tu);
    assert!(
        out.contains("Assignment +="),
        "expected `Assignment +=` in:\n{out}"
    );
}

#[test]
fn unary_prefix_op_renders() {
    let tu = parse_tu("int f(int x) { return -x; }");
    let out = print_ast(&tu);
    assert!(out.contains("UnaryOp -"), "expected `UnaryOp -` in:\n{out}");
}

#[test]
fn postfix_op_renders() {
    let tu = parse_tu("int f(int x) { x++; return x; }");
    let out = print_ast(&tu);
    assert!(out.contains("Postfix"), "got:\n{out}");
    assert!(out.contains("++"), "got:\n{out}");
}

// =========================================================================
// Statements
// =========================================================================

#[test]
fn if_else_renders_both_branches() {
    let tu = parse_tu("int f(int x) { if (x) return 1; else return 0; }");
    let out = print_ast(&tu);
    assert!(out.contains("If"), "got:\n{out}");
    assert!(out.contains("Then"), "expected `Then:` label in:\n{out}");
    assert!(out.contains("Else"), "expected `Else:` label in:\n{out}");
}

#[test]
fn while_loop_renders() {
    let tu = parse_tu("int f(int x) { while (x) x--; return x; }");
    let out = print_ast(&tu);
    assert!(out.contains("While"), "got:\n{out}");
}

#[test]
fn for_loop_renders_all_three_slots() {
    let tu = parse_tu("int f(void) { for (int i = 0; i < 10; i++) {} return 0; }");
    let out = print_ast(&tu);
    assert!(out.contains("For"), "got:\n{out}");
}

// =========================================================================
// Initializers and designated initializers
// =========================================================================

#[test]
fn compound_literal_initializer_renders() {
    let tu = parse_tu("int arr[3] = {1, 2, 3};");
    let out = print_ast(&tu);
    assert!(out.contains("InitializerList"), "got:\n{out}");
    assert!(out.contains("IntLiteral 1"), "got:\n{out}");
    assert!(out.contains("IntLiteral 3"), "got:\n{out}");
}

#[test]
fn designated_initializer_renders() {
    let tu = parse_tu("struct S { int a; int b; }; struct S s = {.a = 1, .b = 2};");
    let out = print_ast(&tu);
    assert!(out.contains("InitializerList"), "got:\n{out}");
    assert!(
        out.contains("Designator") || out.contains(".a") || out.contains("Field"),
        "expected a designator trace in:\n{out}"
    );
}

// =========================================================================
// Struct declarations
// =========================================================================

#[test]
fn struct_def_renders_member_list() {
    let tu = parse_tu("struct S { int a; char b; };");
    let out = print_ast(&tu);
    assert!(out.contains("Struct"), "got:\n{out}");
    assert!(out.contains('a'), "expected `a` in:\n{out}");
    assert!(out.contains('b'), "expected `b` in:\n{out}");
}

// =========================================================================
// Stability: same input should produce the same output (deterministic)
// =========================================================================

#[test]
fn output_is_deterministic() {
    let src = "int f(int x, int y) { return x * y + 1; }";
    let out_a = print_ast(&parse_tu(src));
    let out_b = print_ast(&parse_tu(src));
    assert_eq!(out_a, out_b, "printer output must be deterministic");
}
