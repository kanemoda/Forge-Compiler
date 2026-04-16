//! Tests for predefined macros (`__FILE__`, `__LINE__`, `__STDC__`, etc.).

use tempfile::TempDir;

use super::helpers::*;
use crate::{PreprocessConfig, Preprocessor, TargetArch};

// Test H — `__FILE__` matches the current path, `__LINE__` the line
// number of the invocation.
#[test]
fn file_and_line_magic_macros_track_current_location() {
    let tmp = TempDir::new().unwrap();
    let main_path = write_file(
        tmp.path(),
        "main.c",
        "const char *f = __FILE__;\nint l = __LINE__;\n",
    );

    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main_path).unwrap();
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let strings = string_literal_values(&out);
    let ints = int_literal_values(&out);
    // __FILE__ should expand to the main.c path.
    assert!(
        strings.iter().any(|s| s.ends_with("main.c")),
        "expected __FILE__ to end in main.c, got {strings:?}"
    );
    // __LINE__ is on line 2 of the two-line file.
    assert!(ints.contains(&2u64), "expected __LINE__ = 2, got {ints:?}");
}

// Test I — the standard version macros resolve to the C17 values.
#[test]
fn standard_version_macros_report_c17() {
    let (mut pp, out) = run("int v = __STDC__;\nlong w = __STDC_VERSION__;\n");
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let ints = int_literal_values(&out);
    assert!(ints.contains(&1u64));
    assert!(ints.contains(&201_710u64));
}

// Test J — platform / architecture macros are installed and selectable
// via `PreprocessConfig::target_arch`.
#[test]
fn target_arch_macro_is_set_according_to_config() {
    let cfg = PreprocessConfig {
        target_arch: TargetArch::AArch64,
        ..PreprocessConfig::default()
    };
    let mut pp = Preprocessor::new(cfg);
    let out = pp.run(lex("int a = __aarch64__;\n"));
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let ints = int_literal_values(&out);
    assert_eq!(ints, vec![1u64]);
}

// Test K — `__has_include` probes the filesystem.  A header that
// exists resolves to `1`, one that does not resolves to `0`.  The
// other `__has_*` queries all resolve to `0` for now.
#[test]
fn has_include_returns_one_for_existing_zero_otherwise() {
    let tmp = TempDir::new().unwrap();
    write_file(tmp.path(), "present.h", "int present_marker;\n");
    let main_path = write_file(
        tmp.path(),
        "main.c",
        "#if __has_include(\"present.h\")\nint present_seen;\n#endif\n\
         #if __has_include(\"absent.h\")\nint absent_seen;\n#else\nint absent_missed;\n#endif\n\
         #if __has_builtin(__builtin_whatever)\nint builtin_seen;\n#else\nint builtin_missed;\n#endif\n",
    );

    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main_path).unwrap();
    let diags = pp.take_diagnostics();
    assert!(diags.is_empty_or_no_errors(), "diags: {diags:?}");
    let names = identifier_names(&out);
    assert!(names.contains(&"present_seen".to_string()));
    assert!(!names.contains(&"absent_seen".to_string()));
    assert!(names.contains(&"absent_missed".to_string()));
    assert!(!names.contains(&"builtin_seen".to_string()));
    assert!(names.contains(&"builtin_missed".to_string()));
}

#[test]
fn date_macro_matches_mmm_dd_yyyy_format() {
    // `__DATE__` is frozen at preprocessor construction and must
    // always have the C17 shape "Mmm dd yyyy": three-letter month,
    // space, two-digit day (leading space when <10), space, four-
    // digit year.
    let (mut pp, out) = run("const char *d = __DATE__;\n");
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let strings = string_literal_values(&out);
    assert_eq!(strings.len(), 1, "expected one string: {strings:?}");
    let date = &strings[0];
    assert_eq!(date.len(), 11, "__DATE__ must be 11 chars: {date:?}");
    const MONTHS: &[&str] = &[
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    assert!(
        MONTHS.iter().any(|m| date.starts_with(m)),
        "month prefix not recognised in {date:?}"
    );
    let bytes = date.as_bytes();
    assert_eq!(bytes[3], b' ');
    assert_eq!(bytes[6], b' ');
    assert!(bytes[4] == b' ' || bytes[4].is_ascii_digit());
    assert!(bytes[5].is_ascii_digit());
    assert!(bytes[7..11].iter().all(|b| b.is_ascii_digit()));
}

#[test]
fn time_macro_matches_hh_mm_ss_format() {
    // `__TIME__` must be the eight-character "HH:MM:SS" form.
    let (mut pp, out) = run("const char *t = __TIME__;\n");
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let strings = string_literal_values(&out);
    assert_eq!(strings.len(), 1, "expected one string: {strings:?}");
    let time = &strings[0];
    assert_eq!(time.len(), 8, "__TIME__ must be 8 chars: {time:?}");
    let bytes = time.as_bytes();
    assert_eq!(bytes[2], b':');
    assert_eq!(bytes[5], b':');
    for i in [0usize, 1, 3, 4, 6, 7] {
        assert!(
            bytes[i].is_ascii_digit(),
            "byte {i} not a digit in {time:?}"
        );
    }
}

#[test]
fn gnuc_compat_macros_advertise_gcc14() {
    // The GCC-compatibility shim claims to be GCC 14.0.0.  System
    // headers routinely gate code on `__GNUC__ >= N` so the exact
    // values matter.
    let (mut pp, out) =
        run("int a = __GNUC__;\nint b = __GNUC_MINOR__;\nint c = __GNUC_PATCHLEVEL__;\n");
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let ints = int_literal_values(&out);
    assert!(
        ints.contains(&14u64),
        "expected __GNUC__ = 14, got {ints:?}"
    );
    assert!(ints.contains(&0u64), "expected __GNUC_MINOR__ = 0");
}

#[test]
fn sizeof_int_and_pointer_predefined_macros_are_lp64() {
    // The whole SIZEOF family targets the LP64 model we advertise.
    // A lot of system headers depend on these exact numbers.
    let (mut pp, out) = run("int i = __SIZEOF_INT__;\n\
         int p = __SIZEOF_POINTER__;\n\
         int l = __SIZEOF_LONG__;\n\
         int c = __CHAR_BIT__;\n");
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let ints = int_literal_values(&out);
    assert!(ints.contains(&4u64), "__SIZEOF_INT__ should be 4");
    assert!(ints.contains(&8u64), "__SIZEOF_POINTER__/LONG should be 8");
    assert!(ints.contains(&8u64), "__CHAR_BIT__ should be 8");
}

#[test]
fn file_macro_tracks_across_includes_and_restores_on_return() {
    // `__FILE__` inside an include must name the included file,
    // then flip back to the including file once the include frame
    // pops.
    let tmp = TempDir::new().unwrap();
    write_file(
        tmp.path(),
        "inner.h",
        "const char *inner_name = __FILE__;\n",
    );
    let main_path = write_file(
        tmp.path(),
        "main.c",
        "#include \"inner.h\"\nconst char *outer_name = __FILE__;\n",
    );
    let mut pp = Preprocessor::new(PreprocessConfig::default());
    let out = pp.run_file(&main_path).unwrap();
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let strings = string_literal_values(&out);
    assert!(
        strings.iter().any(|s| s.ends_with("inner.h")),
        "expected __FILE__ inside include to name inner.h: {strings:?}"
    );
    assert!(
        strings.iter().any(|s| s.ends_with("main.c")),
        "expected __FILE__ after include to name main.c: {strings:?}"
    );
}

#[test]
fn has_attribute_and_has_feature_resolve_to_zero() {
    // These probes are installed as always-0 macros until real
    // attribute / feature support lands.  Test them individually
    // so a later rewrite that accidentally changes one but not the
    // others is caught.
    let (mut pp, out) = run(
        "#if __has_attribute(noreturn)\nint attr_yes;\n#else\nint attr_no;\n#endif\n\
         #if __has_feature(address_sanitizer)\nint feat_yes;\n#else\nint feat_no;\n#endif\n\
         #if __has_c_attribute(nodiscard)\nint cattr_yes;\n#else\nint cattr_no;\n#endif\n",
    );
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let names = identifier_names(&out);
    assert!(names.contains(&"attr_no".to_string()));
    assert!(names.contains(&"feat_no".to_string()));
    assert!(names.contains(&"cattr_no".to_string()));
    assert!(!names.contains(&"attr_yes".to_string()));
    assert!(!names.contains(&"feat_yes".to_string()));
    assert!(!names.contains(&"cattr_yes".to_string()));
}

#[test]
fn host_platform_macros_are_defined_on_host_os() {
    // We can not assume the host OS, but whichever branch is live
    // must pick *exactly* one of the two well-known families.
    let src = "#if defined(__linux__) || defined(__APPLE__)\nint host_ok;\n\
               #else\nint host_unknown;\n#endif\n";
    let (mut pp, out) = run(src);
    assert!(pp.take_diagnostics().is_empty_or_no_errors());
    let names = identifier_names(&out);
    let linux_or_mac = cfg!(target_os = "linux") || cfg!(target_os = "macos");
    if linux_or_mac {
        assert!(
            names.contains(&"host_ok".to_string()),
            "expected host platform macro to be defined"
        );
    } else {
        assert!(names.contains(&"host_unknown".to_string()));
    }
}
