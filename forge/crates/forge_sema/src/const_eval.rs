//! Integer constant expression (ICX) evaluator — C17 §6.6.
//!
//! ICX is the narrow form of constant expression that case labels, enum
//! enumerators, array sizes, `_Alignas(N)`, and bit-field widths all
//! require.  It permits:
//!
//! * integer, character, and (evaluating to int) enum constants,
//! * the arithmetic, bitwise, shift, logical, relational, and ternary
//!   operators,
//! * casts to integer types,
//! * `sizeof`/`_Alignof` operands that yield compile-time constants.
//!
//! It forbids side-effectful constructs: assignment, `++` / `--`,
//! function calls, comma, address-of/deref, and compound literals.
//!
//! # Note on general constant expressions
//!
//! C17 §6.6p9 defines a *general* constant expression that also allows
//! address constants — `&x`, `&arr[3]`, `(char*)0 + 5` — for use in
//! static-storage initialisers.  This file does **not** implement that
//! form; Phase 5 IR lowering will handle address constants through a
//! separate path.
//
// TODO(phase5): implement address-constant evaluation for static
// initialisers alongside IR lowering.

use forge_diagnostics::Diagnostic;
use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::Expr;
use forge_parser::ast_ops::{BinaryOp, UnaryOp};

use crate::context::SemaContext;
use crate::resolve::resolve_type_name;
use crate::scope::{SymbolKind, SymbolTable};
use crate::types::{Signedness, TargetInfo, Type};

// =========================================================================
// ConstValue
// =========================================================================

/// The value produced by a constant expression.
///
/// `Unsigned` and `Integer` wrap a `u64`/`i64` bit pattern.  Arithmetic
/// wraps around on overflow, matching C's implementation-defined but
/// conventional two's-complement behaviour on all Forge-supported
/// targets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConstValue {
    /// A signed-integer result.
    Integer(i64),
    /// An unsigned-integer result.
    Unsigned(u64),
    /// A floating-point result.  Forbidden for integer constant
    /// expressions; only appears while evaluating intermediate casts.
    Float(f64),
}

impl ConstValue {
    /// Convert to `i64` if the value is an integer that fits.  Used by
    /// case labels, enum values, and array sizes.
    pub fn to_i64(&self) -> Option<i64> {
        match self {
            ConstValue::Integer(v) => Some(*v),
            ConstValue::Unsigned(v) => i64::try_from(*v).ok(),
            ConstValue::Float(_) => None,
        }
    }

    /// Convert to `u64` if the value is a non-negative integer.  Used by
    /// sizes and bit-field widths.
    pub fn to_u64(&self) -> Option<u64> {
        match self {
            ConstValue::Integer(v) => u64::try_from(*v).ok(),
            ConstValue::Unsigned(v) => Some(*v),
            ConstValue::Float(_) => None,
        }
    }

    /// `true` if the value is arithmetically zero.
    pub fn is_zero(&self) -> bool {
        match self {
            ConstValue::Integer(v) => *v == 0,
            ConstValue::Unsigned(v) => *v == 0,
            ConstValue::Float(v) => *v == 0.0,
        }
    }

    fn as_bool(&self) -> bool {
        !self.is_zero()
    }
}

// =========================================================================
// Entry points
// =========================================================================

/// Evaluate `expr` as an integer constant expression per C17 §6.6.
///
/// Returns `None` and emits at least one diagnostic when the expression
/// is not a valid ICX (contains a function call, references a non-enum
/// variable, etc.).  Signed arithmetic wraps on overflow; division /
/// modulo by zero emit a diagnostic and yield `Some(Integer(0))` so
/// downstream analysis can continue without cascading errors.
pub fn eval_icx(
    expr: &Expr,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<ConstValue> {
    eval(expr, table, target, ctx)
}

/// Convenience wrapper — evaluate as [`i64`].
pub fn eval_icx_as_i64(
    expr: &Expr,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<i64> {
    eval_icx(expr, table, target, ctx)?.to_i64()
}

/// Evaluate `expr` as an integer constant expression, discarding any
/// diagnostics the attempt produces.
///
/// Used by the `__builtin_constant_p` handler to probe whether an
/// expression is compile-time constant without surfacing the reasons
/// it was rejected — those reasons belong to the regular type-checking
/// pass, not to this probe.
pub fn eval_icx_quiet(
    expr: &Expr,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<ConstValue> {
    let saved = ctx.diagnostics.len();
    let result = eval_icx(expr, table, target, ctx);
    ctx.diagnostics.truncate(saved);
    result
}

/// Convenience wrapper — evaluate as non-negative [`u64`].
pub fn eval_icx_as_u64(
    expr: &Expr,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<u64> {
    eval_icx(expr, table, target, ctx)?.to_u64()
}

// =========================================================================
// Core evaluator
// =========================================================================

fn eval(
    expr: &Expr,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<ConstValue> {
    match expr {
        Expr::IntLiteral { value, suffix, .. } => Some(int_literal(*value, *suffix)),

        Expr::CharLiteral { value, .. } => Some(ConstValue::Integer(i64::from(*value))),

        Expr::FloatLiteral { value, .. } => Some(ConstValue::Float(*value)),

        Expr::StringLiteral { span, .. } => {
            ctx.emit(error(
                "string literal is not an integer constant expression",
                *span,
            ));
            None
        }

        Expr::Ident { name, span, .. } => eval_ident(name, *span, table, ctx),

        Expr::UnaryOp {
            op, operand, span, ..
        } => eval_unary(*op, operand, *span, table, target, ctx),

        Expr::BinaryOp {
            op,
            left,
            right,
            span,
            ..
        } => eval_binary(*op, left, right, *span, table, target, ctx),

        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            let cond = eval(condition, table, target, ctx)?;
            if cond.as_bool() {
                eval(then_expr, table, target, ctx)
            } else {
                eval(else_expr, table, target, ctx)
            }
        }

        Expr::Cast {
            type_name,
            expr: inner,
            span,
            ..
        } => {
            let v = eval(inner, table, target, ctx)?;
            let ty = resolve_type_name(type_name, table, target, ctx)?;
            if !ty.ty.is_integer() {
                ctx.emit(error(
                    "cast in a constant expression must target an integer type",
                    *span,
                ));
                return None;
            }
            Some(cast_to_integer(v, &ty.ty, target))
        }

        Expr::SizeofType {
            type_name, span, ..
        } => {
            let ty = resolve_type_name(type_name, table, target, ctx)?;
            match ty.ty.size_of(target, &ctx.type_ctx) {
                Some(n) => Some(ConstValue::Unsigned(n)),
                None => {
                    ctx.emit(error("sizeof on an incomplete or sizeless type", *span));
                    None
                }
            }
        }

        Expr::SizeofExpr { span, .. } => {
            // sizeof(expression) needs full expression type inference,
            // which lands in Phase 4 expression analysis.  Until then we
            // reject it with a clear diagnostic so callers can retry
            // using sizeof(type-name).
            ctx.emit(error(
                "sizeof(expression) in a constant expression is not supported yet",
                *span,
            ));
            None
        }

        Expr::AlignofType {
            type_name, span, ..
        } => {
            let ty = resolve_type_name(type_name, table, target, ctx)?;
            match ty.ty.align_of(target, &ctx.type_ctx) {
                Some(n) => Some(ConstValue::Unsigned(n)),
                None => {
                    ctx.emit(error("_Alignof on a type with no alignment", *span));
                    None
                }
            }
        }

        // -- Everything below is explicitly rejected by C17 §6.6. --
        Expr::Assignment { span, .. } => {
            ctx.emit(error(
                "assignment is not allowed in a constant expression",
                *span,
            ));
            None
        }
        Expr::FunctionCall {
            callee, args, span, ..
        } => {
            if is_builtin_constant_p_callee(callee) {
                return Some(eval_builtin_constant_p(args, table, target, ctx));
            }
            ctx.emit(error(
                "function call is not allowed in a constant expression",
                *span,
            ));
            None
        }
        Expr::Comma { span, .. } => {
            ctx.emit(error(
                "comma operator is not allowed in a constant expression",
                *span,
            ));
            None
        }
        Expr::PostfixOp { span, .. } => {
            ctx.emit(error(
                "postfix '++' / '--' is not allowed in a constant expression",
                *span,
            ));
            None
        }
        Expr::MemberAccess { span, .. } | Expr::ArraySubscript { span, .. } => {
            ctx.emit(error(
                "member or subscript access is not an integer constant expression",
                *span,
            ));
            None
        }
        Expr::CompoundLiteral { span, .. } => {
            ctx.emit(error(
                "compound literal is not a constant expression",
                *span,
            ));
            None
        }
        Expr::GenericSelection { span, .. } => {
            ctx.emit(error(
                "_Generic selection is not supported in a constant expression yet",
                *span,
            ));
            None
        }

        Expr::BuiltinOffsetof {
            ty,
            designator,
            span,
            ..
        } => crate::expr::compute_offsetof_value(ty, designator, *span, table, target, ctx)
            .map(ConstValue::Unsigned),

        Expr::BuiltinTypesCompatibleP { t1, t2, .. } => {
            let a = resolve_type_name(t1, table, target, ctx)?;
            let b = resolve_type_name(t2, table, target, ctx)?;
            let result = crate::types::are_compatible_unqualified(&a, &b, &ctx.type_ctx);
            Some(ConstValue::Integer(i64::from(result)))
        }
    }
}

fn int_literal(value: u64, suffix: IntSuffix) -> ConstValue {
    match suffix {
        IntSuffix::U | IntSuffix::UL | IntSuffix::ULL => ConstValue::Unsigned(value),
        _ => ConstValue::Integer(value as i64),
    }
}

fn is_builtin_constant_p_callee(callee: &Expr) -> bool {
    matches!(callee, Expr::Ident { name, .. } if name == "__builtin_constant_p")
}

/// Evaluate a `__builtin_constant_p(arg)` call inside an ICX context.
///
/// Returns `Integer(1)` if `arg` is itself a valid integer constant
/// expression, else `Integer(0)`.  The probe is silent — diagnostics
/// emitted while trying to evaluate `arg` are discarded so the only
/// observable output is the 0/1 answer.
fn eval_builtin_constant_p(
    args: &[Expr],
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> ConstValue {
    if args.len() != 1 {
        return ConstValue::Integer(0);
    }
    let probe = eval_icx_quiet(&args[0], table, target, ctx);
    ConstValue::Integer(if probe.is_some() { 1 } else { 0 })
}

fn eval_ident(
    name: &str,
    span: Span,
    table: &mut SymbolTable,
    ctx: &mut SemaContext,
) -> Option<ConstValue> {
    match table.lookup(name) {
        Some(sym) => match sym.kind {
            SymbolKind::EnumConstant { value, .. } => Some(ConstValue::Integer(value)),
            _ => {
                ctx.emit(error(
                    format!("'{name}' is not a constant expression"),
                    span,
                ));
                None
            }
        },
        None => {
            ctx.emit(error(format!("undefined identifier '{name}'"), span));
            None
        }
    }
}

// =========================================================================
// Unary / binary arithmetic
// =========================================================================

fn eval_unary(
    op: UnaryOp,
    operand: &Expr,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<ConstValue> {
    match op {
        UnaryOp::Plus => eval(operand, table, target, ctx),
        UnaryOp::Minus => {
            let v = eval(operand, table, target, ctx)?;
            Some(negate(v))
        }
        UnaryOp::BitNot => {
            let v = eval(operand, table, target, ctx)?;
            bitnot(v, span, ctx)
        }
        UnaryOp::LogNot => {
            let v = eval(operand, table, target, ctx)?;
            Some(ConstValue::Integer(i64::from(!v.as_bool())))
        }
        UnaryOp::PreIncrement | UnaryOp::PreDecrement => {
            ctx.emit(error(
                "prefix '++' / '--' is not allowed in a constant expression",
                span,
            ));
            None
        }
        UnaryOp::AddrOf | UnaryOp::Deref => {
            ctx.emit(error(
                "address-of / dereference is not an integer constant expression",
                span,
            ));
            None
        }
    }
}

fn negate(v: ConstValue) -> ConstValue {
    match v {
        ConstValue::Integer(i) => ConstValue::Integer(i.wrapping_neg()),
        ConstValue::Unsigned(u) => ConstValue::Unsigned(u.wrapping_neg()),
        ConstValue::Float(f) => ConstValue::Float(-f),
    }
}

fn bitnot(v: ConstValue, span: Span, ctx: &mut SemaContext) -> Option<ConstValue> {
    match v {
        ConstValue::Integer(i) => Some(ConstValue::Integer(!i)),
        ConstValue::Unsigned(u) => Some(ConstValue::Unsigned(!u)),
        ConstValue::Float(_) => {
            ctx.emit(error("'~' requires an integer operand", span));
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn eval_binary(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<ConstValue> {
    // Short-circuit logical operators first — they evaluate the right
    // operand only when the left does not already decide the result.
    match op {
        BinaryOp::LogAnd => {
            let l = eval(left, table, target, ctx)?;
            if !l.as_bool() {
                return Some(ConstValue::Integer(0));
            }
            let r = eval(right, table, target, ctx)?;
            return Some(ConstValue::Integer(i64::from(r.as_bool())));
        }
        BinaryOp::LogOr => {
            let l = eval(left, table, target, ctx)?;
            if l.as_bool() {
                return Some(ConstValue::Integer(1));
            }
            let r = eval(right, table, target, ctx)?;
            return Some(ConstValue::Integer(i64::from(r.as_bool())));
        }
        _ => {}
    }

    let l = eval(left, table, target, ctx)?;
    let r = eval(right, table, target, ctx)?;

    match op {
        BinaryOp::Add => Some(arith_add(l, r)),
        BinaryOp::Sub => Some(arith_sub(l, r)),
        BinaryOp::Mul => Some(arith_mul(l, r)),
        BinaryOp::Div => arith_div(l, r, span, ctx),
        BinaryOp::Mod => arith_mod(l, r, span, ctx),
        BinaryOp::BitAnd => bitwise(l, r, span, ctx, |a, b| a & b, |a, b| a & b),
        BinaryOp::BitOr => bitwise(l, r, span, ctx, |a, b| a | b, |a, b| a | b),
        BinaryOp::BitXor => bitwise(l, r, span, ctx, |a, b| a ^ b, |a, b| a ^ b),
        BinaryOp::Shl => shift(l, r, span, ctx, /* left = */ true),
        BinaryOp::Shr => shift(l, r, span, ctx, /* left = */ false),
        BinaryOp::Eq => Some(compare(l, r, |o| o == std::cmp::Ordering::Equal)),
        BinaryOp::Ne => Some(compare(l, r, |o| o != std::cmp::Ordering::Equal)),
        BinaryOp::Lt => Some(compare(l, r, |o| o == std::cmp::Ordering::Less)),
        BinaryOp::Gt => Some(compare(l, r, |o| o == std::cmp::Ordering::Greater)),
        BinaryOp::Le => Some(compare(l, r, |o| o != std::cmp::Ordering::Greater)),
        BinaryOp::Ge => Some(compare(l, r, |o| o != std::cmp::Ordering::Less)),
        BinaryOp::LogAnd | BinaryOp::LogOr => unreachable!("handled above"),
    }
}

// --- arithmetic helpers ---

fn promote_pair(l: ConstValue, r: ConstValue) -> (ConstValue, ConstValue) {
    // Any-Float → promote both to Float.
    if matches!(l, ConstValue::Float(_)) || matches!(r, ConstValue::Float(_)) {
        return (to_float(l), to_float(r));
    }
    // Any-Unsigned → promote both to Unsigned (u64 wrap).
    if matches!(l, ConstValue::Unsigned(_)) || matches!(r, ConstValue::Unsigned(_)) {
        return (to_unsigned(l), to_unsigned(r));
    }
    (l, r)
}

fn to_float(v: ConstValue) -> ConstValue {
    match v {
        ConstValue::Integer(i) => ConstValue::Float(i as f64),
        ConstValue::Unsigned(u) => ConstValue::Float(u as f64),
        ConstValue::Float(_) => v,
    }
}

fn to_unsigned(v: ConstValue) -> ConstValue {
    match v {
        ConstValue::Integer(i) => ConstValue::Unsigned(i as u64),
        ConstValue::Unsigned(_) => v,
        ConstValue::Float(f) => ConstValue::Unsigned(f as u64),
    }
}

fn arith_add(l: ConstValue, r: ConstValue) -> ConstValue {
    match promote_pair(l, r) {
        (ConstValue::Integer(a), ConstValue::Integer(b)) => ConstValue::Integer(a.wrapping_add(b)),
        (ConstValue::Unsigned(a), ConstValue::Unsigned(b)) => {
            ConstValue::Unsigned(a.wrapping_add(b))
        }
        (ConstValue::Float(a), ConstValue::Float(b)) => ConstValue::Float(a + b),
        _ => unreachable!("promote_pair aligned both sides"),
    }
}

fn arith_sub(l: ConstValue, r: ConstValue) -> ConstValue {
    match promote_pair(l, r) {
        (ConstValue::Integer(a), ConstValue::Integer(b)) => ConstValue::Integer(a.wrapping_sub(b)),
        (ConstValue::Unsigned(a), ConstValue::Unsigned(b)) => {
            ConstValue::Unsigned(a.wrapping_sub(b))
        }
        (ConstValue::Float(a), ConstValue::Float(b)) => ConstValue::Float(a - b),
        _ => unreachable!(),
    }
}

fn arith_mul(l: ConstValue, r: ConstValue) -> ConstValue {
    match promote_pair(l, r) {
        (ConstValue::Integer(a), ConstValue::Integer(b)) => ConstValue::Integer(a.wrapping_mul(b)),
        (ConstValue::Unsigned(a), ConstValue::Unsigned(b)) => {
            ConstValue::Unsigned(a.wrapping_mul(b))
        }
        (ConstValue::Float(a), ConstValue::Float(b)) => ConstValue::Float(a * b),
        _ => unreachable!(),
    }
}

fn arith_div(
    l: ConstValue,
    r: ConstValue,
    span: Span,
    ctx: &mut SemaContext,
) -> Option<ConstValue> {
    if r.is_zero() {
        ctx.emit(error("division by zero in constant expression", span));
        return Some(ConstValue::Integer(0));
    }
    Some(match promote_pair(l, r) {
        (ConstValue::Integer(a), ConstValue::Integer(b)) => ConstValue::Integer(a.wrapping_div(b)),
        (ConstValue::Unsigned(a), ConstValue::Unsigned(b)) => ConstValue::Unsigned(a / b),
        (ConstValue::Float(a), ConstValue::Float(b)) => ConstValue::Float(a / b),
        _ => unreachable!(),
    })
}

fn arith_mod(
    l: ConstValue,
    r: ConstValue,
    span: Span,
    ctx: &mut SemaContext,
) -> Option<ConstValue> {
    if r.is_zero() {
        ctx.emit(error("modulo by zero in constant expression", span));
        return Some(ConstValue::Integer(0));
    }
    // `%` is integer-only in C.
    let (l, r) = promote_pair(l, r);
    match (l, r) {
        (ConstValue::Integer(a), ConstValue::Integer(b)) => {
            Some(ConstValue::Integer(a.wrapping_rem(b)))
        }
        (ConstValue::Unsigned(a), ConstValue::Unsigned(b)) => Some(ConstValue::Unsigned(a % b)),
        _ => {
            ctx.emit(error("'%' requires integer operands", span));
            None
        }
    }
}

fn bitwise<FI, FU>(
    l: ConstValue,
    r: ConstValue,
    span: Span,
    ctx: &mut SemaContext,
    fi: FI,
    fu: FU,
) -> Option<ConstValue>
where
    FI: FnOnce(i64, i64) -> i64,
    FU: FnOnce(u64, u64) -> u64,
{
    let (l, r) = promote_pair(l, r);
    match (l, r) {
        (ConstValue::Integer(a), ConstValue::Integer(b)) => Some(ConstValue::Integer(fi(a, b))),
        (ConstValue::Unsigned(a), ConstValue::Unsigned(b)) => Some(ConstValue::Unsigned(fu(a, b))),
        _ => {
            ctx.emit(error("bitwise operator requires integer operands", span));
            None
        }
    }
}

fn shift(
    l: ConstValue,
    r: ConstValue,
    span: Span,
    ctx: &mut SemaContext,
    is_left: bool,
) -> Option<ConstValue> {
    // C says the shift amount must be non-negative and less than the
    // width of the promoted left operand.  We wrap the amount mod 64 so
    // that constant-folding never panics; out-of-range shifts are
    // technically UB but not worth a hard error.
    let amount = r.to_i64()?;
    if amount < 0 {
        ctx.emit(error("negative shift count in constant expression", span));
        return Some(ConstValue::Integer(0));
    }
    let amount = (amount as u32) & 63;
    match l {
        ConstValue::Integer(a) => Some(ConstValue::Integer(if is_left {
            a.wrapping_shl(amount)
        } else {
            a.wrapping_shr(amount)
        })),
        ConstValue::Unsigned(a) => Some(ConstValue::Unsigned(if is_left {
            a.wrapping_shl(amount)
        } else {
            a.wrapping_shr(amount)
        })),
        ConstValue::Float(_) => {
            ctx.emit(error("shift requires integer operands", span));
            None
        }
    }
}

fn compare<F: FnOnce(std::cmp::Ordering) -> bool>(
    l: ConstValue,
    r: ConstValue,
    pred: F,
) -> ConstValue {
    use std::cmp::Ordering;
    let ord = match promote_pair(l, r) {
        (ConstValue::Integer(a), ConstValue::Integer(b)) => a.cmp(&b),
        (ConstValue::Unsigned(a), ConstValue::Unsigned(b)) => a.cmp(&b),
        (ConstValue::Float(a), ConstValue::Float(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Less),
        _ => unreachable!(),
    };
    ConstValue::Integer(i64::from(pred(ord)))
}

// =========================================================================
// Integer casts
// =========================================================================

/// Convert `v` to an integer type `ty` with C's normal truncation rules.
fn cast_to_integer(v: ConstValue, ty: &Type, target: &TargetInfo) -> ConstValue {
    let width_bits = match ty.size_of(target, &crate::types::TypeContext::default()) {
        Some(bytes) => (bytes * 8).min(64),
        None => 64,
    };
    let is_unsigned = matches!(
        ty,
        Type::Bool
            | Type::Char {
                signedness: Signedness::Unsigned
            }
            | Type::Short { is_unsigned: true }
            | Type::Int { is_unsigned: true }
            | Type::Long { is_unsigned: true }
            | Type::LongLong { is_unsigned: true }
    );

    let raw: u64 = match v {
        ConstValue::Integer(i) => i as u64,
        ConstValue::Unsigned(u) => u,
        ConstValue::Float(f) => f as i64 as u64,
    };

    let mask = if width_bits == 64 {
        u64::MAX
    } else {
        (1u64 << width_bits) - 1
    };
    let truncated = raw & mask;

    if is_unsigned {
        ConstValue::Unsigned(truncated)
    } else {
        // Sign-extend from `width_bits` to 64 bits.
        let shift = 64 - width_bits;
        let signed = (truncated as i64) << shift >> shift;
        ConstValue::Integer(signed)
    }
}

// =========================================================================
// Diagnostic helper
// =========================================================================

fn error(msg: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::error(msg).span(span.range())
}
