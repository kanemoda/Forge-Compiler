//! Tests for literal type classification (Prompt 4.4).
//!
//! Integer literals pick a type from the C17 §6.4.4.1 candidate list based
//! on suffix and value.  Floating / character / string literals follow the
//! §6.4.4.2–§6.4.5 rules.  Every test here would fail if the literal
//! dispatcher were removed or returned the wrong type.

use forge_lexer::{CharPrefix, FloatSuffix, IntSuffix, Span, StringPrefix};
use forge_parser::ast::Expr;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::expr::{check_expr, check_expr_in_context, ValueContext};
use crate::scope::SymbolTable;
use crate::types::{ArraySize, QualType, Signedness, Type};

use super::helpers::ti;

const S: Span = Span::new(0, 0);

fn int_lit(value: u64, suffix: IntSuffix, id: u32) -> Expr {
    Expr::IntLiteral {
        value,
        suffix,
        span: S,
        node_id: NodeId(id),
    }
}

fn float_lit(value: f64, suffix: FloatSuffix, id: u32) -> Expr {
    Expr::FloatLiteral {
        value,
        suffix,
        span: S,
        node_id: NodeId(id),
    }
}

fn char_lit(value: u32, prefix: CharPrefix, id: u32) -> Expr {
    Expr::CharLiteral {
        value,
        prefix,
        span: S,
        node_id: NodeId(id),
    }
}

fn string_lit(value: &str, prefix: StringPrefix, id: u32) -> Expr {
    Expr::StringLiteral {
        value: value.to_string(),
        prefix,
        span: S,
        node_id: NodeId(id),
    }
}

fn analyze(expr: &Expr) -> (QualType, SemaContext) {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let qt = check_expr(expr, &mut table, &ti(), &mut ctx);
    (qt, ctx)
}

// -------------------------------------------------------------------------
// Integer literals
// -------------------------------------------------------------------------

#[test]
fn int_literal_small_is_int() {
    let (qt, ctx) = analyze(&int_lit(42, IntSuffix::None, 1));
    assert!(!ctx.has_errors());
    assert_eq!(qt.ty, Type::Int { is_unsigned: false });
}

#[test]
fn int_literal_exceeding_int_is_long() {
    // 2^31 + 1 does not fit in a signed int but fits in long on LP64.
    let (qt, _) = analyze(&int_lit(0x80000001, IntSuffix::None, 2));
    assert_eq!(qt.ty, Type::Long { is_unsigned: false });
}

#[test]
fn int_literal_with_u_suffix_is_unsigned_int() {
    let (qt, _) = analyze(&int_lit(1, IntSuffix::U, 3));
    assert_eq!(qt.ty, Type::Int { is_unsigned: true });
}

#[test]
fn int_literal_with_l_suffix_is_long() {
    let (qt, _) = analyze(&int_lit(1, IntSuffix::L, 4));
    assert_eq!(qt.ty, Type::Long { is_unsigned: false });
}

#[test]
fn int_literal_with_ll_suffix_is_long_long() {
    let (qt, _) = analyze(&int_lit(1, IntSuffix::LL, 5));
    assert_eq!(qt.ty, Type::LongLong { is_unsigned: false });
}

#[test]
fn int_literal_with_ul_suffix_is_unsigned_long() {
    let (qt, _) = analyze(&int_lit(1, IntSuffix::UL, 6));
    assert_eq!(qt.ty, Type::Long { is_unsigned: true });
}

#[test]
fn int_literal_with_ull_suffix_is_unsigned_long_long() {
    let (qt, _) = analyze(&int_lit(1, IntSuffix::ULL, 7));
    assert_eq!(qt.ty, Type::LongLong { is_unsigned: true });
}

// -------------------------------------------------------------------------
// Floating literals
// -------------------------------------------------------------------------

#[test]
fn float_literal_no_suffix_is_double() {
    let (qt, _) = analyze(&float_lit(1.5, FloatSuffix::None, 10));
    assert_eq!(qt.ty, Type::Double);
}

#[test]
fn float_literal_f_suffix_is_float() {
    let (qt, _) = analyze(&float_lit(1.5, FloatSuffix::F, 11));
    assert_eq!(qt.ty, Type::Float);
}

#[test]
fn float_literal_l_suffix_is_long_double() {
    let (qt, _) = analyze(&float_lit(1.5, FloatSuffix::L, 12));
    assert_eq!(qt.ty, Type::LongDouble);
}

// -------------------------------------------------------------------------
// Character literals
// -------------------------------------------------------------------------

#[test]
fn char_literal_no_prefix_is_int() {
    let (qt, _) = analyze(&char_lit(u32::from(b'a'), CharPrefix::None, 20));
    assert_eq!(qt.ty, Type::Int { is_unsigned: false });
}

#[test]
fn char_literal_wide_is_wchar_t() {
    let (qt, _) = analyze(&char_lit(0x41, CharPrefix::L, 21));
    // wchar_t on x86-64 Linux is int.
    assert_eq!(qt.ty, Type::Int { is_unsigned: false });
}

#[test]
fn char_literal_u16_is_unsigned_short() {
    let (qt, _) = analyze(&char_lit(0x41, CharPrefix::U16, 22));
    assert_eq!(qt.ty, Type::Short { is_unsigned: true });
}

#[test]
fn char_literal_u32_is_unsigned_int() {
    let (qt, _) = analyze(&char_lit(0x41, CharPrefix::U32, 23));
    assert_eq!(qt.ty, Type::Int { is_unsigned: true });
}

// -------------------------------------------------------------------------
// String literals
// -------------------------------------------------------------------------

#[test]
fn string_literal_no_prefix_is_char_array_with_nul() {
    // "hi" → char[3] (two chars plus the terminating NUL).  Inspect the
    // type in a non-decaying context so the array shape is preserved.
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let qt = check_expr_in_context(
        &string_lit("hi", StringPrefix::None, 30),
        ValueContext::SizeofOperand,
        &mut table,
        &ti(),
        &mut ctx,
    );
    match qt.ty {
        Type::Array { element, size } => {
            assert_eq!(
                element.ty,
                Type::Char {
                    signedness: Signedness::Plain
                }
            );
            assert_eq!(size, ArraySize::Fixed(3));
        }
        other => panic!("expected char[3], got {other:?}"),
    }
}

#[test]
fn string_literal_wide_is_wchar_t_array_with_nul() {
    // L"AB" → wchar_t[3] (two wide chars plus the NUL).
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let qt = check_expr_in_context(
        &string_lit("AB", StringPrefix::L, 31),
        ValueContext::SizeofOperand,
        &mut table,
        &ti(),
        &mut ctx,
    );
    match qt.ty {
        Type::Array { element, size } => {
            assert_eq!(element.ty, Type::Int { is_unsigned: false });
            assert_eq!(size, ArraySize::Fixed(3));
        }
        other => panic!("expected wchar_t[3], got {other:?}"),
    }
}
