//! Stress tests for extreme-but-legal inputs.
//!
//! These exercise the sema pipeline with shapes that are vanishingly
//! rare in real code but must still type-check without errors.  Each
//! test drives the full lexer → parser → sema pipeline through
//! [`super::helpers::analyze_source`].
//!
//! Any test that fails is documented in `phases/phase_04_report.md`
//! with a severity assessment.

use super::helpers::{analyze_source, assert_source_clean};

#[test]
fn stress_100_locals_in_one_function() {
    // 100 distinct `int lN;` declarations each followed by an
    // assignment using that name.  All must land in the symbol table
    // and the uses must resolve without "undeclared identifier".
    let mut body = String::new();
    for i in 0..100 {
        body.push_str(&format!("    int l{i} = {i};\n"));
    }
    for i in 0..100 {
        body.push_str(&format!("    l{i} = l{i} + 1;\n"));
    }
    let src = format!("int main(void) {{\n{body}    return 0;\n}}\n");
    assert_source_clean(&src);
}

#[test]
fn stress_50_nested_scopes() {
    // 50 nested `{{ ... }}` with a fresh local at each depth shadowing
    // the outer name.  Lookup must see the innermost at every level.
    let mut opens = String::new();
    let mut closes = String::new();
    for _ in 0..50 {
        opens.push_str("{ int x = 0; (void)x; ");
        closes.push('}');
    }
    let src = format!("int main(void) {{\n{opens}{closes}\n    return 0;\n}}\n");
    assert_source_clean(&src);
}

#[test]
fn stress_struct_with_50_members() {
    let mut members = String::new();
    for i in 0..50 {
        members.push_str(&format!("    int m{i};\n"));
    }
    let src = format!(
        "struct Big {{\n{members}}};\n\
         int main(void) {{\n    struct Big b;\n    b.m0 = 1;\n    b.m49 = 50;\n    return 0;\n}}\n"
    );
    assert_source_clean(&src);
}

#[test]
fn stress_function_with_20_parameters() {
    let params: Vec<String> = (0..20).map(|i| format!("int p{i}")).collect();
    let proto = params.join(", ");
    let uses: Vec<String> = (0..20).map(|i| format!("p{i}")).collect();
    let body = format!("    return {};\n", uses.join(" + "));
    let src = format!(
        "int f({proto}) {{\n{body}}}\nint main(void) {{ return f({ones}); }}\n",
        ones = (0..20).map(|_| "0").collect::<Vec<_>>().join(", ")
    );
    assert_source_clean(&src);
}

#[test]
fn stress_10_levels_pointer_indirection() {
    // Declare `int **********p;` (10 stars), dereference once to get
    // `int *********`, and chain dereferences in an expression that
    // type-checks.
    let src = r#"
        int main(void) {
            int v = 7;
            int *p1 = &v;
            int **p2 = &p1;
            int ***p3 = &p2;
            int ****p4 = &p3;
            int *****p5 = &p4;
            int ******p6 = &p5;
            int *******p7 = &p6;
            int ********p8 = &p7;
            int *********p9 = &p8;
            int **********p10 = &p9;
            return **********p10;
        }
    "#;
    assert_source_clean(src);
}

#[test]
fn stress_expression_tree_depth_100() {
    let mut expr = String::from("1");
    for _ in 0..99 {
        expr.push_str(" + 1");
    }
    let src = format!("int main(void) {{ return {expr}; }}\n");
    assert_source_clean(&src);
}

#[test]
fn stress_enum_with_100_constants() {
    let mut body = String::new();
    for i in 0..100 {
        body.push_str(&format!("    E{i} = {i},\n"));
    }
    let src = format!(
        "enum E {{\n{body}}};\n\
         int main(void) {{ return (int)E99; }}\n"
    );
    assert_source_clean(&src);
}

#[test]
fn stress_multiple_tu_extern() {
    // Two separate analyze_translation_unit calls referencing the same
    // extern name.  Each is its own TU; the second must NOT conflict
    // with symbols from the first.  (Each analyze_translation_unit
    // call constructs a fresh SymbolTable.)
    let (a_diags, _a_ctx, _a_table) = analyze_source(
        r#"
            extern int shared_counter;
            int bump_a(void) { shared_counter = shared_counter + 1; return shared_counter; }
        "#,
    );
    let (b_diags, _b_ctx, _b_table) = analyze_source(
        r#"
            extern int shared_counter;
            int bump_b(void) { shared_counter = shared_counter + 2; return shared_counter; }
        "#,
    );
    let no_errors = |ds: &Vec<forge_diagnostics::Diagnostic>| {
        ds.iter()
            .all(|d| !matches!(d.severity, forge_diagnostics::Severity::Error))
    };
    assert!(no_errors(&a_diags), "TU A had errors: {a_diags:?}");
    assert!(no_errors(&b_diags), "TU B had errors: {b_diags:?}");
}

#[test]
fn stress_typedef_chain_depth_10() {
    let src = r#"
        typedef int A0;
        typedef A0 A1;
        typedef A1 A2;
        typedef A2 A3;
        typedef A3 A4;
        typedef A4 A5;
        typedef A5 A6;
        typedef A6 A7;
        typedef A7 A8;
        typedef A8 A9;
        int main(void) { A9 x = 42; return x; }
    "#;
    assert_source_clean(src);
}

#[test]
fn stress_mutually_recursive_struct_3way() {
    // struct A → B → C → A, all through pointers so every layout is
    // complete once the final forward-reference resolves.
    let src = r#"
        struct A;
        struct B;
        struct C;
        struct A { struct B *b; };
        struct B { struct C *c; };
        struct C { struct A *a; };
        int main(void) {
            struct A a;
            struct B b;
            struct C c;
            a.b = &b;
            b.c = &c;
            c.a = &a;
            return 0;
        }
    "#;
    assert_source_clean(src);
}

#[test]
fn stress_anonymous_union_inside_struct_inside_union() {
    // Deep anonymous flattening — member names from the innermost
    // anonymous union must be reachable from the outermost union.
    let src = r#"
        union Outer {
            int tag;
            struct {
                int header;
                union {
                    int inner_a;
                    int inner_b;
                };
            };
        };
        int main(void) {
            union Outer o;
            o.tag = 0;
            o.header = 1;
            o.inner_a = 2;
            o.inner_b = 3;
            return o.inner_b;
        }
    "#;
    assert_source_clean(src);
}

#[test]
fn stress_generic_with_20_associations() {
    // `_Generic(x, T1: v1, T2: v2, ..., default: vD)` with 20 arms.
    // Selection of `int` branch must succeed.
    let mut arms = String::new();
    // Use distinct integer / pointer-to-distinct-struct types to make
    // every arm type-unique and avoid C11's duplicate-type error.
    let types = [
        "char",
        "signed char",
        "unsigned char",
        "short",
        "unsigned short",
        "unsigned int",
        "long",
        "unsigned long",
        "long long",
        "unsigned long long",
        "float",
        "double",
        "long double",
        "void *",
        "char *",
        "short *",
        "long *",
        "float *",
        "double *",
    ];
    for (i, t) in types.iter().enumerate() {
        arms.push_str(&format!("            {t}: {i},\n"));
    }
    // The 20th selection arm: plain `int` — the branch we want chosen.
    arms.push_str("            int: 999,\n");
    let src = format!(
        "int main(void) {{\n\
            int x = 0;\n\
            return _Generic(x,\n{arms}            default: -1);\n\
        }}\n"
    );
    assert_source_clean(&src);
}
