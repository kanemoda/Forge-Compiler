//! Unknown `__attribute__((...))` names must be silently ignored.
//!
//! GCC's contract — which glibc and many third-party headers depend on —
//! is that unrecognised attribute names produce a warning at worst and
//! never an error.  Forge currently takes the stricter-than-GCC-but-
//! still-reasonable stance of neither warning nor erroring: the parser
//! accepts the syntax and sema never inspects the attribute list for
//! unknown names.

use super::helpers::analyze_source;
use forge_diagnostics::Severity;

fn assert_no_errors_or_warnings(src: &str) {
    let (diags, _ctx, _table) = analyze_source(src);
    let loud: Vec<_> = diags
        .iter()
        .filter(|d| matches!(d.severity, Severity::Error | Severity::Warning))
        .collect();
    assert!(
        loud.is_empty(),
        "expected no errors or warnings, got: {loud:?}"
    );
}

#[test]
fn bare_unknown_attribute_accepted() {
    assert_no_errors_or_warnings(
        "__attribute__((zorgblatt)) int x;
         int main(void) { return x; }",
    );
}

#[test]
fn unknown_attribute_with_args_accepted() {
    assert_no_errors_or_warnings(
        r#"
            __attribute__((zorgblatt(1, 2, "three"))) int y;
            int main(void) { return y; }
        "#,
    );
}

#[test]
fn mixed_known_and_unknown_attributes_accepted() {
    // glibc prototypes sometimes stack several attributes — sema must
    // not choke on a mix of ones we care about (like noreturn) with
    // ones we ignore (like `deprecated`, `unused`, `format`).
    assert_no_errors_or_warnings(
        "__attribute__((deprecated, unused, format(printf, 1, 2))) void f(const char *, ...);
         int main(void) { (void)f; return 0; }",
    );
}

#[test]
fn unknown_attribute_on_struct_member_accepted() {
    assert_no_errors_or_warnings(
        "struct S {
             int x __attribute__((bananas));
         };
         int main(void) {
             struct S s;
             s.x = 0;
             return s.x;
         }",
    );
}
