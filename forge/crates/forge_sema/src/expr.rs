//! Expression type checking — Prompts 4.4 and 4.5.
//!
//! This module annotates every expression node with a resolved type,
//! marks lvalues, and records the implicit conversions applied at each
//! site (array-to-pointer decay, function-to-pointer decay,
//! lvalue-to-rvalue, integer promotion, the usual arithmetic
//! conversions, float↔int, pointer↔null, etc.).  Prompt 4.4 handles
//! literals, identifiers, member access, subscript, sizeof, and
//! `_Alignof`.  Prompt 4.5 extends the dispatcher to cover every
//! remaining expression form — arithmetic, comparison, logical,
//! shift, assignment, increment/decrement, address-of / dereference,
//! function calls, casts, ternary, compound literal, `_Generic`, and
//! comma.

use forge_diagnostics::Diagnostic;
use forge_lexer::{CharPrefix, FloatSuffix, IntSuffix, Span, StringPrefix};
use forge_parser::ast::{Expr, GenericAssociation, Initializer, TypeName};
use forge_parser::ast_ops::{AssignOp, BinaryOp, PostfixOp, UnaryOp};
use forge_parser::NodeId;

use crate::context::SemaContext;
use crate::declare::check_initializer;
use crate::resolve::resolve_type_name;
use crate::scope::{SymbolKind, SymbolTable};
use crate::types::{
    are_compatible_unqualified, integer_promotion, is_null_pointer_constant,
    usual_arithmetic_conversions, ArraySize, ImplicitConversion, MemberLayout, QualType,
    Signedness, SizeofKind, TargetInfo, Type,
};

// =========================================================================
// ValueContext
// =========================================================================

/// The syntactic context in which an expression appears.
///
/// Default conversions (lvalue-to-rvalue, array-to-pointer,
/// function-to-pointer) only kick in for some contexts; for example,
/// `sizeof(arr)` must see the array type, not a decayed pointer.  Every
/// sub-expression visit threads a [`ValueContext`] to suppress the
/// appropriate conversions.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ValueContext {
    /// Default — all default conversions apply.
    RValue,
    /// Operand of unary `&` — lvalue-to-rvalue and array/function decay
    /// are all suppressed so the expression's raw type is preserved.
    AddressOf,
    /// Operand of `sizeof` / `_Alignof` — lvalue-to-rvalue and decays
    /// are all suppressed.
    SizeofOperand,
    /// LHS of an assignment — lvalue-to-rvalue is suppressed so the
    /// result is a storage target, but array / function decay would be
    /// a type error elsewhere anyway.
    AssignmentLhs,
    /// Operand of prefix / postfix `++` / `--` — lvalue-to-rvalue is
    /// suppressed for the same reason.
    IncrementOperand,
}

impl ValueContext {
    fn applies_lvalue_to_rvalue(self) -> bool {
        matches!(self, ValueContext::RValue)
    }

    fn applies_decay(self) -> bool {
        !matches!(self, ValueContext::AddressOf | ValueContext::SizeofOperand)
    }
}

// =========================================================================
// Entry points
// =========================================================================

/// Type-check `expr` in the default (rvalue) context.
///
/// Returns the expression's type *after* default conversions have been
/// applied; the conversion itself (if any) is recorded on the AST node
/// in [`SemaContext::implicit_convs`].  The raw type (before conversion)
/// is **not** separately stored; callers who need to distinguish should
/// inspect `implicit_convs` or call [`check_expr_in_context`] with a
/// context that suppresses the conversion.
pub fn check_expr(
    expr: &Expr,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    check_expr_in_context(expr, ValueContext::RValue, table, target, ctx)
}

/// Type-check `expr` in an explicit value context.
pub fn check_expr_in_context(
    expr: &Expr,
    vctx: ValueContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let raw = check_expr_raw(expr, table, target, ctx);
    let converted = apply_default_conversions(expr_node_id(expr), raw, vctx, ctx);
    ctx.set_type(expr_node_id(expr), converted.clone());
    converted
}

// =========================================================================
// Raw dispatcher
// =========================================================================

fn check_expr_raw(
    expr: &Expr,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    match expr {
        Expr::IntLiteral { value, suffix, .. } => {
            QualType::unqualified(int_literal_type(*value, *suffix))
        }
        Expr::FloatLiteral { suffix, .. } => QualType::unqualified(float_literal_type(*suffix)),
        Expr::CharLiteral { prefix, .. } => {
            QualType::unqualified(char_literal_type(*prefix, target))
        }
        Expr::StringLiteral {
            value,
            prefix,
            node_id,
            ..
        } => {
            let qt = string_literal_type(value, *prefix, target);
            ctx.mark_lvalue(*node_id);
            qt
        }
        Expr::Ident {
            name,
            span,
            node_id,
            ..
        } => check_ident(name, *span, *node_id, table, ctx),
        Expr::MemberAccess {
            object,
            member,
            is_arrow,
            span,
            node_id,
            ..
        } => check_member_access(
            object, member, *is_arrow, *span, *node_id, table, target, ctx,
        ),
        Expr::ArraySubscript {
            array,
            index,
            span,
            node_id,
            ..
        } => check_subscript(array, index, *span, *node_id, table, target, ctx),
        Expr::SizeofExpr {
            expr: inner,
            span,
            node_id,
            ..
        } => check_sizeof_expr(inner, *span, *node_id, table, target, ctx),
        Expr::SizeofType {
            type_name,
            span,
            node_id,
            ..
        } => check_sizeof_type(type_name, *span, *node_id, table, target, ctx),
        Expr::AlignofType {
            type_name, span, ..
        } => check_alignof_type(type_name, *span, table, target, ctx),
        Expr::UnaryOp {
            op,
            operand,
            span,
            node_id,
        } => check_unary_op(*op, operand, *span, *node_id, table, target, ctx),
        Expr::PostfixOp {
            op,
            operand,
            span,
            node_id,
        } => check_postfix_op(*op, operand, *span, *node_id, table, target, ctx),
        Expr::BinaryOp {
            op,
            left,
            right,
            span,
            node_id,
        } => check_binary_op(*op, left, right, *span, *node_id, table, target, ctx),
        Expr::Assignment {
            op,
            target: lhs,
            value,
            span,
            node_id,
        } => check_assignment(*op, lhs, value, *span, *node_id, table, target, ctx),
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            span,
            node_id,
        } => check_ternary(
            condition, then_expr, else_expr, *span, *node_id, table, target, ctx,
        ),
        Expr::FunctionCall {
            callee,
            args,
            span,
            node_id,
        } => check_call(callee, args, *span, *node_id, table, target, ctx),
        Expr::Cast {
            type_name,
            expr: inner,
            span,
            node_id,
        } => check_cast(type_name, inner, *span, *node_id, table, target, ctx),
        Expr::CompoundLiteral {
            type_name,
            initializer,
            span,
            node_id,
        } => check_compound_literal(type_name, initializer, *span, *node_id, table, target, ctx),
        Expr::GenericSelection {
            controlling,
            associations,
            span,
            node_id,
        } => check_generic(
            controlling,
            associations,
            *span,
            *node_id,
            table,
            target,
            ctx,
        ),
        Expr::Comma {
            exprs,
            span,
            node_id,
        } => check_comma(exprs, *span, *node_id, table, target, ctx),
        Expr::BuiltinOffsetof {
            ty,
            designator,
            span,
            node_id,
        } => check_builtin_offsetof(ty, designator, *span, *node_id, table, target, ctx),
        Expr::BuiltinTypesCompatibleP {
            t1,
            t2,
            span,
            node_id,
        } => check_builtin_types_compatible_p(t1, t2, *span, *node_id, table, target, ctx),
    }
}

// =========================================================================
// Default conversions
// =========================================================================

fn apply_default_conversions(
    node_id: NodeId,
    raw: QualType,
    vctx: ValueContext,
    ctx: &mut SemaContext,
) -> QualType {
    // Array-to-pointer decay has priority over lvalue-to-rvalue because
    // C17 §6.3.2.1 says array conversion happens before the value is
    // read.  The conversion produces an rvalue pointer regardless of
    // whether the array itself was an lvalue.
    if let Type::Array { element, .. } = &raw.ty {
        if vctx.applies_decay() {
            let pointee = (**element).clone();
            let decayed = QualType::unqualified(Type::Pointer {
                pointee: Box::new(pointee),
            });
            ctx.set_implicit_conv(node_id, ImplicitConversion::ArrayToPointer);
            return decayed;
        }
        return raw;
    }
    if let Type::Function { .. } = &raw.ty {
        if vctx.applies_decay() {
            let decayed = QualType::unqualified(Type::Pointer {
                pointee: Box::new(raw),
            });
            ctx.set_implicit_conv(node_id, ImplicitConversion::FunctionToPointer);
            return decayed;
        }
        return raw;
    }
    // Lvalue-to-rvalue: only for scalar-ish lvalues that the context
    // actually reads from.
    if ctx.is_lvalue(node_id) && vctx.applies_lvalue_to_rvalue() {
        ctx.set_implicit_conv(node_id, ImplicitConversion::LvalueToRvalue);
        let mut rvalue = raw;
        rvalue.is_const = false;
        rvalue.is_volatile = false;
        rvalue.is_restrict = false;
        rvalue.is_atomic = false;
        return rvalue;
    }
    raw
}

// =========================================================================
// Literals
// =========================================================================

fn int_literal_type(value: u64, suffix: IntSuffix) -> Type {
    // Candidate widths fixed for the x86-64 Linux LP64 target (int = 4,
    // long = long long = 8).  For genuinely multi-target support this
    // function would instead consult TargetInfo, but LP64 is the only
    // target Forge ships.
    let int_max = i32::MAX as u64;
    let uint_max = u32::MAX as u64;
    let long_max = i64::MAX as u64;
    match suffix {
        IntSuffix::None => {
            if value <= int_max {
                Type::Int { is_unsigned: false }
            } else if value <= long_max {
                Type::Long { is_unsigned: false }
            } else {
                Type::LongLong { is_unsigned: false }
            }
        }
        IntSuffix::U => {
            if value <= uint_max {
                Type::Int { is_unsigned: true }
            } else {
                Type::Long { is_unsigned: true }
            }
        }
        IntSuffix::L => {
            if value <= long_max {
                Type::Long { is_unsigned: false }
            } else {
                Type::LongLong { is_unsigned: false }
            }
        }
        IntSuffix::UL => Type::Long { is_unsigned: true },
        IntSuffix::LL => {
            if value <= long_max {
                Type::LongLong { is_unsigned: false }
            } else {
                Type::LongLong { is_unsigned: true }
            }
        }
        IntSuffix::ULL => Type::LongLong { is_unsigned: true },
    }
}

fn float_literal_type(suffix: FloatSuffix) -> Type {
    match suffix {
        FloatSuffix::F => Type::Float,
        FloatSuffix::L => Type::LongDouble,
        FloatSuffix::None => Type::Double,
    }
}

fn char_literal_type(prefix: CharPrefix, target: &TargetInfo) -> Type {
    match prefix {
        CharPrefix::None => Type::Int { is_unsigned: false },
        CharPrefix::L => target.wchar_t_type(),
        CharPrefix::U16 => Type::Short { is_unsigned: true },
        CharPrefix::U32 => Type::Int { is_unsigned: true },
    }
}

fn string_literal_type(value: &str, prefix: StringPrefix, target: &TargetInfo) -> QualType {
    let (elem_ty, length) = match prefix {
        StringPrefix::None | StringPrefix::Utf8 => (
            Type::Char {
                signedness: Signedness::Plain,
            },
            value.len() as u64 + 1,
        ),
        StringPrefix::L => (target.wchar_t_type(), value.chars().count() as u64 + 1),
        StringPrefix::U16 => (
            Type::Short { is_unsigned: true },
            value.chars().count() as u64 + 1,
        ),
        StringPrefix::U32 => (
            Type::Int { is_unsigned: true },
            value.chars().count() as u64 + 1,
        ),
    };
    QualType::unqualified(Type::Array {
        element: Box::new(QualType::unqualified(elem_ty)),
        size: ArraySize::Fixed(length),
    })
}

// =========================================================================
// Identifier
// =========================================================================

fn check_ident(
    name: &str,
    span: Span,
    node_id: NodeId,
    table: &mut SymbolTable,
    ctx: &mut SemaContext,
) -> QualType {
    let Some(sym) = table.lookup(name) else {
        ctx.emit(error(format!("undefined identifier '{name}'"), span));
        return QualType::unqualified(Type::Int { is_unsigned: false });
    };
    let kind = sym.kind.clone();
    let ty = sym.ty.clone();
    let id = sym.id;
    ctx.set_symbol_ref(node_id, id);
    match kind {
        SymbolKind::Variable | SymbolKind::Parameter => {
            ctx.mark_lvalue(node_id);
            ty
        }
        SymbolKind::Function => ty,
        SymbolKind::EnumConstant { .. } => {
            // C17 §6.4.4.3 — enumeration constants have type `int`.
            QualType::unqualified(Type::Int { is_unsigned: false })
        }
        SymbolKind::Typedef => {
            ctx.emit(error(
                format!("'{name}' is a typedef name, not a value"),
                span,
            ));
            QualType::unqualified(Type::Int { is_unsigned: false })
        }
    }
}

// =========================================================================
// Member access
// =========================================================================

#[allow(clippy::too_many_arguments)]
fn check_member_access(
    object: &Expr,
    member: &str,
    is_arrow: bool,
    span: Span,
    node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let obj_ty = check_expr_in_context(object, ValueContext::RValue, table, target, ctx);
    let object_was_lvalue = ctx.is_lvalue(expr_node_id(object));

    // Determine the struct/union type and any qualifiers that should
    // propagate to the member result.
    let (agg_ty, obj_quals_const, obj_quals_volatile): (Type, bool, bool) = if is_arrow {
        match obj_ty.ty {
            Type::Pointer { pointee } => {
                (pointee.ty.clone(), pointee.is_const, pointee.is_volatile)
            }
            _ => {
                ctx.emit(error(
                    "left operand of '->' must be a pointer to a struct or union",
                    span,
                ));
                return QualType::unqualified(Type::Int { is_unsigned: false });
            }
        }
    } else {
        (obj_ty.ty.clone(), obj_ty.is_const, obj_ty.is_volatile)
    };

    let members = match &agg_ty {
        Type::Struct(sid) => ctx.type_ctx.struct_layout(*sid).map(|l| l.members.clone()),
        Type::Union(uid) => ctx.type_ctx.union_layout(*uid).map(|l| l.members.clone()),
        _ => {
            ctx.emit(error(
                format!(
                    "left operand of '{}' is not a struct or union",
                    if is_arrow { "->" } else { "." }
                ),
                span,
            ));
            return QualType::unqualified(Type::Int { is_unsigned: false });
        }
    };

    let Some(members) = members else {
        ctx.emit(error(
            format!("member access on incomplete type — no member '{member}'"),
            span,
        ));
        return QualType::unqualified(Type::Int { is_unsigned: false });
    };

    let Some(mut member_ty) = lookup_member_type(&members, member) else {
        ctx.emit(error(format!("no member named '{member}'"), span));
        return QualType::unqualified(Type::Int { is_unsigned: false });
    };

    // Propagate qualifiers from the aggregate expression (C17 §6.5.2.3p3).
    if obj_quals_const {
        member_ty.is_const = true;
    }
    if obj_quals_volatile {
        member_ty.is_volatile = true;
    }

    // `->` always yields an lvalue; `.` yields one iff the object is an
    // lvalue.
    if is_arrow || object_was_lvalue {
        ctx.mark_lvalue(node_id);
    }

    member_ty
}

fn lookup_member_type(members: &[MemberLayout], name: &str) -> Option<QualType> {
    for m in members {
        if let Some(n) = &m.name {
            if n == name {
                return Some(m.ty.clone());
            }
        } else if let Some(anon) = &m.anon_members {
            if let Some((_off, ty)) = anon.fields.get(name) {
                return Some(ty.clone());
            }
        }
    }
    None
}

// =========================================================================
// Subscript
// =========================================================================

fn check_subscript(
    array: &Expr,
    index: &Expr,
    span: Span,
    node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let a_ty = check_expr_in_context(array, ValueContext::RValue, table, target, ctx);
    let i_ty = check_expr_in_context(index, ValueContext::RValue, table, target, ctx);

    let (ptr_qt, idx_qt) = if a_ty.ty.is_pointer() {
        (a_ty, i_ty)
    } else if i_ty.ty.is_pointer() {
        // C17 allows the reversed form `0[arr]`.
        (i_ty, a_ty)
    } else {
        ctx.emit(error("subscript requires a pointer and an integer", span));
        return QualType::unqualified(Type::Int { is_unsigned: false });
    };

    if !idx_qt.ty.is_integer() {
        ctx.emit(error("array subscript must be an integer", span));
    }

    let Type::Pointer { pointee } = ptr_qt.ty else {
        unreachable!("is_pointer check just succeeded");
    };

    ctx.mark_lvalue(node_id);
    *pointee
}

// =========================================================================
// sizeof
// =========================================================================

fn check_sizeof_expr(
    inner: &Expr,
    span: Span,
    node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let inner_ty = check_expr_in_context(inner, ValueContext::SizeofOperand, table, target, ctx);
    finish_sizeof(&inner_ty.ty, node_id, span, target, ctx);
    QualType::unqualified(target.size_t_type())
}

fn check_sizeof_type(
    tn: &TypeName,
    span: Span,
    node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    if let Some(ty) = resolve_type_name(tn, table, target, ctx) {
        finish_sizeof(&ty.ty, node_id, span, target, ctx);
    }
    QualType::unqualified(target.size_t_type())
}

fn finish_sizeof(
    ty: &Type,
    node_id: NodeId,
    span: Span,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    if ty.is_function() {
        ctx.emit(error("sizeof applied to a function type", span));
        return;
    }
    if type_involves_vla(ty) {
        // C17 §6.5.3.4p2 — sizeof of a VLA is evaluated at runtime.
        // Expression tracking for the VLA dimensions is Phase 5 work;
        // for now we just record the kind so lowering knows not to
        // constant-fold.
        ctx.set_sizeof_kind(
            node_id,
            SizeofKind::RuntimeVla {
                expr_nodes: Vec::new(),
            },
        );
        return;
    }
    if !ty.is_complete(&ctx.type_ctx) {
        ctx.emit(error("sizeof applied to an incomplete type", span));
        return;
    }
    if let Some(n) = ty.size_of(target, &ctx.type_ctx) {
        ctx.set_sizeof_kind(node_id, SizeofKind::Constant(n));
    }
}

fn type_involves_vla(ty: &Type) -> bool {
    match ty {
        Type::Array { element, size } => {
            matches!(size, ArraySize::Variable) || type_involves_vla(&element.ty)
        }
        _ => false,
    }
}

// =========================================================================
// _Alignof
// =========================================================================

fn check_alignof_type(
    tn: &TypeName,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    if let Some(ty) = resolve_type_name(tn, table, target, ctx) {
        if ty.ty.is_function() {
            ctx.emit(error("_Alignof applied to a function type", span));
        } else if !ty.ty.is_complete(&ctx.type_ctx) && !ty.ty.is_array() {
            // Arrays of incomplete size are fine for alignof — alignment
            // only depends on the element type.
            ctx.emit(error("_Alignof applied to an incomplete type", span));
        }
    }
    QualType::unqualified(target.size_t_type())
}

// =========================================================================
// Unary operators
// =========================================================================

fn check_unary_op(
    op: UnaryOp,
    operand: &Expr,
    span: Span,
    node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    match op {
        UnaryOp::AddrOf => check_addr_of(operand, span, table, target, ctx),
        UnaryOp::Deref => check_deref(operand, span, node_id, table, target, ctx),
        UnaryOp::Plus | UnaryOp::Minus => check_unary_arith(op, operand, span, table, target, ctx),
        UnaryOp::BitNot => check_unary_bitnot(operand, span, table, target, ctx),
        UnaryOp::LogNot => check_unary_lognot(operand, span, table, target, ctx),
        UnaryOp::PreIncrement | UnaryOp::PreDecrement => {
            check_incdec(operand, span, true, table, target, ctx)
        }
    }
}

fn check_postfix_op(
    op: PostfixOp,
    operand: &Expr,
    span: Span,
    _node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let _ = op; // PostfixOp is always ++/--; distinction handled by lowering.
    check_incdec(operand, span, false, table, target, ctx)
}

fn check_addr_of(
    operand: &Expr,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let raw = check_expr_in_context(operand, ValueContext::AddressOf, table, target, ctx);
    // C17 §6.5.3.2p1: operand must be an lvalue (or a function designator,
    // which never fires here because functions decay only in RValue contexts).
    let inner_is_lvalue = ctx.is_lvalue(expr_node_id(operand));
    let is_function = matches!(raw.ty, Type::Function { .. });
    if !inner_is_lvalue && !is_function {
        ctx.emit(error(
            "operand of '&' must be an lvalue or function designator",
            span,
        ));
    }
    QualType::unqualified(Type::Pointer {
        pointee: Box::new(raw),
    })
}

fn check_deref(
    operand: &Expr,
    span: Span,
    node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let ptr_ty = check_expr_in_context(operand, ValueContext::RValue, table, target, ctx);
    match ptr_ty.ty {
        Type::Pointer { pointee } => {
            let pointee_ty = *pointee;
            // C17 §6.3.2.1p1 — a function designator is NOT an lvalue.
            // Dereferencing a function pointer yields the function
            // designator; default conversions will re-apply
            // FunctionToPointer at the outer context.
            if !matches!(pointee_ty.ty, Type::Function { .. }) {
                ctx.mark_lvalue(node_id);
            }
            pointee_ty
        }
        _ => {
            ctx.emit(error("operand of unary '*' must be a pointer", span));
            QualType::unqualified(Type::Int { is_unsigned: false })
        }
    }
}

fn check_unary_arith(
    op: UnaryOp,
    operand: &Expr,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let qt = check_expr_in_context(operand, ValueContext::RValue, table, target, ctx);
    if !qt.ty.is_arithmetic() {
        ctx.emit(error(
            format!(
                "operand of unary '{}' must be of arithmetic type",
                match op {
                    UnaryOp::Plus => "+",
                    UnaryOp::Minus => "-",
                    _ => "?",
                }
            ),
            span,
        ));
        return QualType::unqualified(Type::Int { is_unsigned: false });
    }
    let promoted = integer_promotion(&qt.ty, target);
    if qt.ty.is_integer() && promoted != qt.ty {
        ctx.set_implicit_conv(
            expr_node_id(operand),
            ImplicitConversion::IntegerPromotion {
                to: promoted.clone(),
            },
        );
    }
    QualType::unqualified(promoted)
}

fn check_unary_bitnot(
    operand: &Expr,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let qt = check_expr_in_context(operand, ValueContext::RValue, table, target, ctx);
    if !qt.ty.is_integer() {
        ctx.emit(error("operand of unary '~' must be of integer type", span));
        return QualType::unqualified(Type::Int { is_unsigned: false });
    }
    let promoted = integer_promotion(&qt.ty, target);
    if promoted != qt.ty {
        ctx.set_implicit_conv(
            expr_node_id(operand),
            ImplicitConversion::IntegerPromotion {
                to: promoted.clone(),
            },
        );
    }
    QualType::unqualified(promoted)
}

fn check_unary_lognot(
    operand: &Expr,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let qt = check_expr_in_context(operand, ValueContext::RValue, table, target, ctx);
    if !qt.ty.is_scalar() {
        ctx.emit(error("operand of unary '!' must be of scalar type", span));
    }
    QualType::unqualified(Type::Int { is_unsigned: false })
}

fn check_incdec(
    operand: &Expr,
    span: Span,
    is_prefix: bool,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let _ = is_prefix; // Same semantic type for pre- and post-.
    let qt = check_expr_in_context(operand, ValueContext::IncrementOperand, table, target, ctx);
    let operand_is_lvalue = ctx.is_lvalue(expr_node_id(operand));
    if !operand_is_lvalue {
        ctx.emit(error(
            "operand of '++' or '--' must be a modifiable lvalue",
            span,
        ));
    } else if qt.is_const {
        ctx.emit(error(
            "cannot increment/decrement a const-qualified lvalue",
            span,
        ));
    } else if !(qt.ty.is_arithmetic() || qt.ty.is_pointer()) {
        ctx.emit(error(
            "operand of '++' or '--' must be arithmetic or pointer",
            span,
        ));
    } else if qt.ty.is_pointer() {
        if let Type::Pointer { pointee } = &qt.ty {
            if matches!(pointee.ty, Type::Function { .. }) || !pointee.ty.is_complete(&ctx.type_ctx)
            {
                ctx.emit(error(
                    "arithmetic on pointer to incomplete or function type",
                    span,
                ));
            }
        }
    }
    // Result is the operand's (unqualified) type and is NOT an lvalue.
    QualType::unqualified(qt.ty)
}

// =========================================================================
// Binary operators
// =========================================================================

#[allow(clippy::too_many_arguments)]
fn check_binary_op(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    span: Span,
    node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    match op {
        BinaryOp::Add => check_additive(left, right, true, span, table, target, ctx),
        BinaryOp::Sub => check_additive(left, right, false, span, table, target, ctx),
        BinaryOp::Mul | BinaryOp::Div => check_mul_div(op, left, right, span, table, target, ctx),
        BinaryOp::Mod => check_mod(left, right, span, table, target, ctx),
        BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
            check_bitwise(op, left, right, span, table, target, ctx)
        }
        BinaryOp::Shl | BinaryOp::Shr => check_shift(op, left, right, span, table, target, ctx),
        BinaryOp::LogAnd | BinaryOp::LogOr => {
            check_logical(op, left, right, span, node_id, table, target, ctx)
        }
        BinaryOp::Eq | BinaryOp::Ne => check_equality(op, left, right, span, table, target, ctx),
        BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge => {
            check_relational(op, left, right, span, table, target, ctx)
        }
    }
}

fn check_additive(
    left: &Expr,
    right: &Expr,
    is_add: bool,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let l = check_expr_in_context(left, ValueContext::RValue, table, target, ctx);
    let r = check_expr_in_context(right, ValueContext::RValue, table, target, ctx);

    // arithmetic + arithmetic — usual arithmetic conversions.
    if l.ty.is_arithmetic() && r.ty.is_arithmetic() {
        return QualType::unqualified(balance_arithmetic(left, right, &l.ty, &r.ty, target, ctx));
    }

    // Pointer arithmetic.
    let l_ptr = l.ty.is_pointer();
    let r_ptr = r.ty.is_pointer();
    if is_add {
        // ptr + int / int + ptr → ptr (pointee must be complete, warning
        // for void*).
        if l_ptr && r.ty.is_integer() {
            warn_if_void_ptr_arith(&l.ty, span, ctx);
            return l;
        }
        if r_ptr && l.ty.is_integer() {
            warn_if_void_ptr_arith(&r.ty, span, ctx);
            return r;
        }
        if l_ptr && r_ptr {
            ctx.emit(error("cannot add two pointers", span));
            return QualType::unqualified(Type::Int { is_unsigned: false });
        }
    } else {
        // ptr - int
        if l_ptr && r.ty.is_integer() {
            warn_if_void_ptr_arith(&l.ty, span, ctx);
            return l;
        }
        // ptr - ptr → ptrdiff_t
        if l_ptr && r_ptr {
            let compatible_pointee = match (&l.ty, &r.ty) {
                (Type::Pointer { pointee: lp }, Type::Pointer { pointee: rp }) => {
                    are_compatible_unqualified(lp, rp, &ctx.type_ctx)
                }
                _ => false,
            };
            if !compatible_pointee {
                ctx.emit(error(
                    "pointer subtraction requires compatible pointee types",
                    span,
                ));
            }
            return QualType::unqualified(target.ptrdiff_t_type());
        }
    }

    ctx.emit(error(
        "operands have invalid types for this arithmetic operator",
        span,
    ));
    QualType::unqualified(Type::Int { is_unsigned: false })
}

fn check_mul_div(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let l = check_expr_in_context(left, ValueContext::RValue, table, target, ctx);
    let r = check_expr_in_context(right, ValueContext::RValue, table, target, ctx);
    if !(l.ty.is_arithmetic() && r.ty.is_arithmetic()) {
        ctx.emit(error(
            format!(
                "operands of '{}' must be arithmetic",
                match op {
                    BinaryOp::Mul => "*",
                    BinaryOp::Div => "/",
                    _ => "?",
                }
            ),
            span,
        ));
        return QualType::unqualified(Type::Int { is_unsigned: false });
    }
    QualType::unqualified(balance_arithmetic(left, right, &l.ty, &r.ty, target, ctx))
}

fn check_mod(
    left: &Expr,
    right: &Expr,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let l = check_expr_in_context(left, ValueContext::RValue, table, target, ctx);
    let r = check_expr_in_context(right, ValueContext::RValue, table, target, ctx);
    if !(l.ty.is_integer() && r.ty.is_integer()) {
        ctx.emit(error("operands of '%' must be integer", span));
        return QualType::unqualified(Type::Int { is_unsigned: false });
    }
    QualType::unqualified(balance_arithmetic(left, right, &l.ty, &r.ty, target, ctx))
}

fn check_bitwise(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let l = check_expr_in_context(left, ValueContext::RValue, table, target, ctx);
    let r = check_expr_in_context(right, ValueContext::RValue, table, target, ctx);
    if !(l.ty.is_integer() && r.ty.is_integer()) {
        ctx.emit(error(
            format!(
                "operands of '{}' must be integer",
                match op {
                    BinaryOp::BitAnd => "&",
                    BinaryOp::BitOr => "|",
                    BinaryOp::BitXor => "^",
                    _ => "?",
                }
            ),
            span,
        ));
        return QualType::unqualified(Type::Int { is_unsigned: false });
    }
    QualType::unqualified(balance_arithmetic(left, right, &l.ty, &r.ty, target, ctx))
}

fn check_shift(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let l = check_expr_in_context(left, ValueContext::RValue, table, target, ctx);
    let r = check_expr_in_context(right, ValueContext::RValue, table, target, ctx);
    if !(l.ty.is_integer() && r.ty.is_integer()) {
        ctx.emit(error(
            format!(
                "operands of '{}' must be integer",
                match op {
                    BinaryOp::Shl => "<<",
                    BinaryOp::Shr => ">>",
                    _ => "?",
                }
            ),
            span,
        ));
        return QualType::unqualified(Type::Int { is_unsigned: false });
    }
    // C17 §6.5.7p3 — integer promotion applies to each operand
    // independently, and the result type is the promoted LEFT type.
    let l_promoted = integer_promotion(&l.ty, target);
    let r_promoted = integer_promotion(&r.ty, target);
    if l_promoted != l.ty {
        ctx.set_implicit_conv(
            expr_node_id(left),
            ImplicitConversion::IntegerPromotion {
                to: l_promoted.clone(),
            },
        );
    }
    if r_promoted != r.ty {
        ctx.set_implicit_conv(
            expr_node_id(right),
            ImplicitConversion::IntegerPromotion {
                to: r_promoted.clone(),
            },
        );
    }
    // Constant shift-count warning if RHS is an integer literal wider
    // than or equal to the LHS promoted-type bit width.
    if let Expr::IntLiteral { value, .. } = right {
        let lhs_bits = integer_bit_width(&l_promoted, target);
        if lhs_bits > 0 && *value >= lhs_bits {
            ctx.emit(warning(
                format!("shift count ({value}) >= width of type ({lhs_bits} bits)"),
                span,
            ));
        }
    }
    QualType::unqualified(l_promoted)
}

#[allow(clippy::too_many_arguments)]
fn check_logical(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    span: Span,
    _node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let _ = op;
    let l = check_expr_in_context(left, ValueContext::RValue, table, target, ctx);
    let r = check_expr_in_context(right, ValueContext::RValue, table, target, ctx);
    if !l.ty.is_scalar() || !r.ty.is_scalar() {
        ctx.emit(error("operands of '&&' / '||' must be scalar", span));
    }
    QualType::unqualified(Type::Int { is_unsigned: false })
}

fn check_equality(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let _ = op;
    let l = check_expr_in_context(left, ValueContext::RValue, table, target, ctx);
    let r = check_expr_in_context(right, ValueContext::RValue, table, target, ctx);

    if l.ty.is_arithmetic() && r.ty.is_arithmetic() {
        balance_arithmetic(left, right, &l.ty, &r.ty, target, ctx);
        return QualType::unqualified(Type::Int { is_unsigned: false });
    }

    let l_ptr = l.ty.is_pointer();
    let r_ptr = r.ty.is_pointer();
    let l_null = is_null_pointer_constant(&l.ty, const_value_of(left));
    let r_null = is_null_pointer_constant(&r.ty, const_value_of(right));

    if (l_ptr && (r_ptr || r_null)) || (r_ptr && l_null) {
        return QualType::unqualified(Type::Int { is_unsigned: false });
    }

    ctx.emit(error("invalid operands to equality comparison", span));
    QualType::unqualified(Type::Int { is_unsigned: false })
}

fn check_relational(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let _ = op;
    let l = check_expr_in_context(left, ValueContext::RValue, table, target, ctx);
    let r = check_expr_in_context(right, ValueContext::RValue, table, target, ctx);

    if l.ty.is_arithmetic() && r.ty.is_arithmetic() {
        balance_arithmetic(left, right, &l.ty, &r.ty, target, ctx);
        return QualType::unqualified(Type::Int { is_unsigned: false });
    }

    if l.ty.is_pointer() && r.ty.is_pointer() {
        let compatible = match (&l.ty, &r.ty) {
            (Type::Pointer { pointee: lp }, Type::Pointer { pointee: rp }) => {
                let l_void = matches!(lp.ty, Type::Void);
                let r_void = matches!(rp.ty, Type::Void);
                if l_void || r_void {
                    ctx.emit(warning(
                        "ordering comparison involving 'void *' is implementation-defined",
                        span,
                    ));
                    true
                } else {
                    are_compatible_unqualified(lp, rp, &ctx.type_ctx)
                }
            }
            _ => false,
        };
        if !compatible {
            ctx.emit(error(
                "ordered comparison of pointers to incompatible types",
                span,
            ));
        }
        return QualType::unqualified(Type::Int { is_unsigned: false });
    }

    ctx.emit(error("invalid operands to relational comparison", span));
    QualType::unqualified(Type::Int { is_unsigned: false })
}

// Apply usual arithmetic conversions to two operands, recording the
// implicit conversion on each operand node where needed.  The returned
// value is the common arithmetic type.
fn balance_arithmetic(
    left: &Expr,
    right: &Expr,
    l_ty: &Type,
    r_ty: &Type,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Type {
    let common = usual_arithmetic_conversions(l_ty, r_ty, target);
    record_arith_conv(expr_node_id(left), l_ty, &common, ctx);
    record_arith_conv(expr_node_id(right), r_ty, &common, ctx);
    common
}

fn record_arith_conv(node_id: NodeId, from: &Type, to: &Type, ctx: &mut SemaContext) {
    if from == to {
        return;
    }
    let conv = match (from.is_floating(), to.is_floating()) {
        (true, true) => ImplicitConversion::FloatConversion { to: to.clone() },
        (false, true) => ImplicitConversion::IntToFloat { to: to.clone() },
        (true, false) => ImplicitConversion::FloatToInt { to: to.clone() },
        (false, false) => ImplicitConversion::ArithmeticConversion { to: to.clone() },
    };
    ctx.set_implicit_conv(node_id, conv);
}

fn warn_if_void_ptr_arith(ptr_ty: &Type, span: Span, ctx: &mut SemaContext) {
    if let Type::Pointer { pointee } = ptr_ty {
        if matches!(pointee.ty, Type::Void) {
            ctx.emit(warning(
                "arithmetic on a pointer to void is a GNU extension",
                span,
            ));
        }
    }
}

fn integer_bit_width(ty: &Type, target: &TargetInfo) -> u64 {
    match ty {
        Type::Bool => target.bool_size * 8,
        Type::Char { .. } => target.char_size * 8,
        Type::Short { .. } => target.short_size * 8,
        Type::Int { .. } | Type::Enum(_) => target.int_size * 8,
        Type::Long { .. } => target.long_size * 8,
        Type::LongLong { .. } => target.long_long_size * 8,
        _ => 0,
    }
}

// Best-effort inspection of literal integer values for null-pointer-
// constant detection.  Returns None for non-literal expressions; callers
// fall back to "not a null pointer constant" when None is returned.
fn const_value_of(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::IntLiteral { value, .. } => i64::try_from(*value).ok(),
        Expr::CharLiteral { value, .. } => Some(i64::from(*value)),
        Expr::Cast { expr: inner, .. } => const_value_of(inner),
        _ => None,
    }
}

// =========================================================================
// Assignment
// =========================================================================

#[allow(clippy::too_many_arguments)]
fn check_assignment(
    op: AssignOp,
    lhs: &Expr,
    rhs: &Expr,
    span: Span,
    _node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let l_ty = check_expr_in_context(lhs, ValueContext::AssignmentLhs, table, target, ctx);
    let r_ty = check_expr_in_context(rhs, ValueContext::RValue, table, target, ctx);

    // LHS must be a modifiable lvalue.
    let lhs_is_lvalue = ctx.is_lvalue(expr_node_id(lhs));
    if !lhs_is_lvalue {
        ctx.emit(error(
            "left-hand side of assignment must be an lvalue",
            span,
        ));
    } else if l_ty.is_const {
        ctx.emit(error("cannot assign to a const-qualified lvalue", span));
    } else if matches!(l_ty.ty, Type::Array { .. }) {
        ctx.emit(error("array type is not assignable", span));
    }

    // Compound assignment must also satisfy the underlying binary op's
    // operand rules — we check assignability of the RHS against the LHS
    // type, and for the operator-specific check we only do a coarse
    // arithmetic/pointer gate here.  The IR lowering expands the op.
    if matches!(
        op,
        AssignOp::ModAssign
            | AssignOp::ShlAssign
            | AssignOp::ShrAssign
            | AssignOp::BitAndAssign
            | AssignOp::BitOrAssign
            | AssignOp::BitXorAssign
    ) && !(l_ty.ty.is_integer() && r_ty.ty.is_integer())
    {
        ctx.emit(error("compound-assignment requires integer operands", span));
    }

    // Assignability checks.
    check_assignability(&l_ty, &r_ty, rhs, span, ctx);

    // Result is LHS type UNQUALIFIED, not lvalue.
    let mut result = l_ty;
    result.is_const = false;
    result.is_volatile = false;
    result.is_restrict = false;
    result.is_atomic = false;
    result
}

/// Check that `rhs` is assignable to `lhs` under C17's simple-assignment
/// rules (§6.5.16.1).  Emits a diagnostic on incompatibility and records
/// the appropriate [`ImplicitConversion`] on `rhs_expr`'s node when the
/// assignment is legal but requires a representation change.
///
/// Shared between the `=` / compound-assignment operator (see
/// [`check_assignment`]) and scalar initialiser checking in
/// [`crate::declare`], so the two surfaces can't drift.
pub(crate) fn check_assignability(
    lhs: &QualType,
    rhs: &QualType,
    rhs_expr: &Expr,
    span: Span,
    ctx: &mut SemaContext,
) {
    // arithmetic ← arithmetic
    if lhs.ty.is_arithmetic() && rhs.ty.is_arithmetic() {
        record_arith_conv(expr_node_id(rhs_expr), &rhs.ty, &lhs.ty, ctx);
        return;
    }
    // _Bool ← scalar
    if matches!(lhs.ty, Type::Bool) && rhs.ty.is_scalar() {
        if rhs.ty.is_pointer() {
            ctx.set_implicit_conv(expr_node_id(rhs_expr), ImplicitConversion::PointerToBoolean);
        }
        return;
    }
    // pointer ← compatible pointer / void* / null
    if lhs.ty.is_pointer() {
        // Null pointer constant
        if is_null_pointer_constant(&rhs.ty, const_value_of(rhs_expr)) {
            ctx.set_implicit_conv(
                expr_node_id(rhs_expr),
                ImplicitConversion::NullPointerConversion,
            );
            return;
        }
        if rhs.ty.is_pointer() {
            if let (Type::Pointer { pointee: lp }, Type::Pointer { pointee: rp }) =
                (&lhs.ty, &rhs.ty)
            {
                let void_involved = matches!(lp.ty, Type::Void) || matches!(rp.ty, Type::Void);
                if !void_involved && !are_compatible_unqualified(lp, rp, &ctx.type_ctx) {
                    ctx.emit(error("assignment from incompatible pointer type", span));
                    return;
                }
                // Qualification: RHS pointee qualifiers must be a subset
                // of LHS pointee qualifiers.
                if (rp.is_const && !lp.is_const)
                    || (rp.is_volatile && !lp.is_volatile)
                    || (rp.is_restrict && !lp.is_restrict)
                    || (rp.is_atomic && !lp.is_atomic)
                {
                    ctx.emit(error(
                        "assignment discards qualifiers from pointer target type",
                        span,
                    ));
                    return;
                }
                let lp_quals = (lp.is_const, lp.is_volatile, lp.is_restrict, lp.is_atomic);
                let rp_quals = (rp.is_const, rp.is_volatile, rp.is_restrict, rp.is_atomic);
                if lp_quals != rp_quals {
                    ctx.set_implicit_conv(
                        expr_node_id(rhs_expr),
                        ImplicitConversion::QualificationConversion,
                    );
                }
                return;
            }
        }
        ctx.emit(error(
            "assignment requires a pointer or null pointer constant on the right",
            span,
        ));
        return;
    }
    // struct/union ← same struct/union
    if lhs.ty.is_struct_or_union() {
        if lhs.ty == rhs.ty {
            return;
        }
        ctx.emit(error("struct/union assignment requires the same tag", span));
        return;
    }
    // Fallback for error recovery — only arithmetic/pointer/struct are
    // legal destinations.  If nothing matched, emit a generic error.
    ctx.emit(error("assignment from an incompatible type", span));
}

// =========================================================================
// Function calls
// =========================================================================

fn check_call(
    callee: &Expr,
    args: &[Expr],
    span: Span,
    _node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    // __builtin_constant_p(expr) — special-cased so the argument is not
    // subjected to assignability against a function prototype: the
    // builtin accepts any expression type.  Type-check the callee to
    // keep its node in the side table, then hand off to the dedicated
    // handler.
    if matches!(callee, Expr::Ident { name, .. } if name == "__builtin_constant_p") {
        let _ = check_expr_in_context(callee, ValueContext::RValue, table, target, ctx);
        return check_builtin_constant_p(args, span, table, target, ctx);
    }

    let callee_ty = check_expr_in_context(callee, ValueContext::RValue, table, target, ctx);
    // After default conversions, a function designator becomes a pointer
    // to function.  Extract the function type.
    let Type::Pointer { pointee } = callee_ty.ty else {
        ctx.emit(error("called object is not a pointer to a function", span));
        for a in args {
            let _ = check_expr_in_context(a, ValueContext::RValue, table, target, ctx);
        }
        return QualType::unqualified(Type::Int { is_unsigned: false });
    };
    let Type::Function {
        return_type,
        params,
        is_variadic,
        is_prototype,
    } = pointee.ty
    else {
        ctx.emit(error("called object is not a pointer to a function", span));
        for a in args {
            let _ = check_expr_in_context(a, ValueContext::RValue, table, target, ctx);
        }
        return QualType::unqualified(Type::Int { is_unsigned: false });
    };

    if is_prototype {
        let fixed = params.len();
        if is_variadic {
            if args.len() < fixed {
                ctx.emit(error(
                    format!(
                        "too few arguments to variadic function (expected at least {fixed}, got {})",
                        args.len()
                    ),
                    span,
                ));
            }
        } else if args.len() != fixed {
            ctx.emit(error(
                format!(
                    "wrong number of arguments to function (expected {fixed}, got {})",
                    args.len()
                ),
                span,
            ));
        }
        for (i, a) in args.iter().enumerate() {
            let a_ty = check_expr_in_context(a, ValueContext::RValue, table, target, ctx);
            if i < fixed {
                check_assignability(&params[i].ty, &a_ty, a, span, ctx);
            } else {
                apply_default_arg_promotion(a, &a_ty.ty, target, ctx);
            }
        }
    } else {
        ctx.emit(warning(
            "call through an unprototyped function declaration",
            span,
        ));
        for a in args {
            let a_ty = check_expr_in_context(a, ValueContext::RValue, table, target, ctx);
            apply_default_arg_promotion(a, &a_ty.ty, target, ctx);
        }
    }

    // C17 §6.5.2.2p5 — strip top-level qualifiers from the return type.
    let mut result = (*return_type).clone();
    result.is_const = false;
    result.is_volatile = false;
    result.is_restrict = false;
    result.is_atomic = false;
    result
}

/// Type-check a `__builtin_constant_p(arg)` call.
///
/// The builtin is modelled as accepting any single expression and always
/// returning `int` (0 or 1).  No type-compatibility check is run on the
/// argument — the whole point of the builtin is to interrogate an
/// expression of arbitrary type.  The constant-folded value is produced
/// by [`crate::const_eval::eval_icx`] when the call appears inside an
/// integer constant expression context.
fn check_builtin_constant_p(
    args: &[Expr],
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    if args.len() != 1 {
        ctx.emit(error(
            format!(
                "__builtin_constant_p takes exactly one argument, got {}",
                args.len()
            ),
            span,
        ));
    }
    for a in args {
        let _ = check_expr_in_context(a, ValueContext::RValue, table, target, ctx);
    }
    QualType::unqualified(Type::Int { is_unsigned: false })
}

// =========================================================================
// __builtin_offsetof and __builtin_types_compatible_p
// =========================================================================

/// Walk an `offsetof` designator against the resolved aggregate type,
/// accumulating byte offsets.  Returns `Some(offset)` on success and
/// records any error-severity diagnostic on mismatch.
///
/// Shared by [`check_builtin_offsetof`] and [`crate::const_eval`]'s
/// equivalent arm so the two surfaces can't drift.
pub(crate) fn compute_offsetof_value(
    ty: &TypeName,
    designator: &[forge_parser::ast::OffsetofMember],
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<u64> {
    use forge_parser::ast::OffsetofMember;

    let resolved = resolve_type_name(ty, table, target, ctx)?;
    let mut current_ty = resolved.ty;
    let mut offset: u64 = 0;

    for step in designator {
        match step {
            OffsetofMember::Field(name) => {
                let members = match &current_ty {
                    Type::Struct(sid) => {
                        ctx.type_ctx.struct_layout(*sid).map(|l| l.members.clone())
                    }
                    Type::Union(uid) => ctx.type_ctx.union_layout(*uid).map(|l| l.members.clone()),
                    _ => {
                        ctx.emit(error("offsetof requires a struct or union type", span));
                        return None;
                    }
                };
                let Some(members) = members else {
                    ctx.emit(error("offsetof on an incomplete aggregate type", span));
                    return None;
                };
                let Some((step_offset, step_ty)) = lookup_offsetof_step(&members, name) else {
                    ctx.emit(error(format!("no member named '{name}'"), span));
                    return None;
                };
                offset = offset.saturating_add(step_offset);
                current_ty = step_ty;
            }
            OffsetofMember::Subscript(idx_expr) => {
                let Type::Array { element, .. } = &current_ty else {
                    ctx.emit(error("subscript applied to non-array in offsetof", span));
                    return None;
                };
                let element_ty = element.ty.clone();
                let index = crate::const_eval::eval_icx_as_i64(idx_expr, table, target, ctx)?;
                if index < 0 {
                    ctx.emit(error("offsetof subscript must be non-negative", span));
                    return None;
                }
                let Some(elem_size) = element_ty.size_of(target, &ctx.type_ctx) else {
                    ctx.emit(error("offsetof subscript element has no known size", span));
                    return None;
                };
                let step_offset = (index as u64).saturating_mul(elem_size);
                offset = offset.saturating_add(step_offset);
                current_ty = element_ty;
            }
        }
    }
    Some(offset)
}

fn lookup_offsetof_step(members: &[MemberLayout], name: &str) -> Option<(u64, Type)> {
    for m in members {
        if let Some(n) = &m.name {
            if n == name {
                return Some((m.offset, m.ty.ty.clone()));
            }
        } else if let Some(anon) = &m.anon_members {
            if let Some((inner_off, ty)) = anon.fields.get(name) {
                return Some((m.offset.saturating_add(*inner_off), ty.ty.clone()));
            }
        }
    }
    None
}

fn check_builtin_offsetof(
    ty: &TypeName,
    designator: &[forge_parser::ast::OffsetofMember],
    span: Span,
    _node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    // Run the offset walk purely for its side-effect of emitting
    // diagnostics on bad designators; the integer value itself lands in
    // the side-table through `eval_icx` when the expression is used in
    // an ICX context.
    let _ = compute_offsetof_value(ty, designator, span, table, target, ctx);
    QualType::unqualified(target.size_t_type())
}

fn check_builtin_types_compatible_p(
    t1: &TypeName,
    t2: &TypeName,
    _span: Span,
    _node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    // Resolve both operands so any ill-formed typedef / tag reference
    // emits its own diagnostic.  The compatibility check itself happens
    // in `eval_icx`; here we just require both sides to type-check.
    let _ = resolve_type_name(t1, table, target, ctx);
    let _ = resolve_type_name(t2, table, target, ctx);
    QualType::unqualified(Type::Int { is_unsigned: false })
}

fn apply_default_arg_promotion(
    arg: &Expr,
    arg_ty: &Type,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    // float → double.
    if matches!(arg_ty, Type::Float) {
        ctx.set_implicit_conv(
            expr_node_id(arg),
            ImplicitConversion::FloatConversion { to: Type::Double },
        );
        return;
    }
    // integer promotions for small integer types.
    if arg_ty.is_integer() {
        let promoted = integer_promotion(arg_ty, target);
        if promoted != *arg_ty {
            ctx.set_implicit_conv(
                expr_node_id(arg),
                ImplicitConversion::IntegerPromotion { to: promoted },
            );
        }
    }
}

// =========================================================================
// Cast
// =========================================================================

fn check_cast(
    tn: &TypeName,
    inner: &Expr,
    span: Span,
    _node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let inner_ty = check_expr_in_context(inner, ValueContext::RValue, table, target, ctx);
    let Some(target_ty) = resolve_type_name(tn, table, target, ctx) else {
        return QualType::unqualified(Type::Int { is_unsigned: false });
    };

    // C17 §6.5.4p2 — `(void)expr` is the explicit-discard cast and is
    // legal for any expression regardless of its type.  This must be
    // checked BEFORE the struct/union source rejection below, otherwise
    // `(void)struct_lvalue` is wrongly diagnosed as a bad source type.
    if matches!(target_ty.ty, Type::Void) {
        return target_ty;
    }

    // Reject illegal target kinds.
    if matches!(target_ty.ty, Type::Array { .. }) {
        ctx.emit(error("cannot cast to an array type", span));
        return target_ty;
    }
    if matches!(target_ty.ty, Type::Function { .. }) {
        ctx.emit(error("cannot cast to a function type", span));
        return target_ty;
    }
    if target_ty.ty.is_struct_or_union() {
        ctx.emit(error("cannot cast to a struct or union type", span));
        return target_ty;
    }
    if inner_ty.ty.is_struct_or_union() {
        ctx.emit(error("cannot cast from a struct or union type", span));
        return target_ty;
    }

    // Record a best-effort implicit conversion so lowering has a hint.
    let from = &inner_ty.ty;
    let to = &target_ty.ty;
    match (
        from.is_arithmetic(),
        to.is_arithmetic(),
        from.is_pointer(),
        to.is_pointer(),
    ) {
        (true, true, _, _) => {
            record_arith_conv(expr_node_id(inner), from, to, ctx);
        }
        (true, false, _, true) if from.is_integer() => {
            ctx.set_implicit_conv(expr_node_id(inner), ImplicitConversion::IntegerToPointer);
        }
        (false, true, true, _) if to.is_integer() => {
            ctx.set_implicit_conv(expr_node_id(inner), ImplicitConversion::PointerToInteger);
        }
        (_, _, true, true) => {}
        _ => {}
    }

    target_ty
}

// =========================================================================
// Ternary
// =========================================================================

#[allow(clippy::too_many_arguments)]
fn check_ternary(
    condition: &Expr,
    then_expr: &Expr,
    else_expr: &Expr,
    span: Span,
    _node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let c_ty = check_expr_in_context(condition, ValueContext::RValue, table, target, ctx);
    if !c_ty.ty.is_scalar() {
        ctx.emit(error("ternary condition must be of scalar type", span));
    }
    let a_ty = check_expr_in_context(then_expr, ValueContext::RValue, table, target, ctx);
    let b_ty = check_expr_in_context(else_expr, ValueContext::RValue, table, target, ctx);

    // Both arithmetic — usual conversions.
    if a_ty.ty.is_arithmetic() && b_ty.ty.is_arithmetic() {
        let common = balance_arithmetic(then_expr, else_expr, &a_ty.ty, &b_ty.ty, target, ctx);
        return QualType::unqualified(common);
    }
    if matches!(a_ty.ty, Type::Void) && matches!(b_ty.ty, Type::Void) {
        return QualType::unqualified(Type::Void);
    }
    // Same struct/union.
    if a_ty.ty.is_struct_or_union() && a_ty.ty == b_ty.ty {
        return a_ty;
    }
    // Pointer cases.
    let a_null = is_null_pointer_constant(&a_ty.ty, const_value_of(then_expr));
    let b_null = is_null_pointer_constant(&b_ty.ty, const_value_of(else_expr));
    if a_ty.ty.is_pointer() || b_ty.ty.is_pointer() {
        let (ptr, other, ptr_is_a) = if a_ty.ty.is_pointer() {
            (&a_ty, &b_ty, true)
        } else {
            (&b_ty, &a_ty, false)
        };
        // pointer + null → pointer
        if ptr.ty.is_pointer() && ((ptr_is_a && b_null) || (!ptr_is_a && a_null)) {
            return ptr.clone();
        }
        // pointer + pointer
        if other.ty.is_pointer() {
            if let (Type::Pointer { pointee: ap }, Type::Pointer { pointee: bp }) =
                (&a_ty.ty, &b_ty.ty)
            {
                let a_void = matches!(ap.ty, Type::Void);
                let b_void = matches!(bp.ty, Type::Void);
                if a_void || b_void {
                    // union of qualifiers on the void pointee
                    let pointee = QualType {
                        ty: Type::Void,
                        is_const: ap.is_const || bp.is_const,
                        is_volatile: ap.is_volatile || bp.is_volatile,
                        is_restrict: ap.is_restrict || bp.is_restrict,
                        is_atomic: ap.is_atomic || bp.is_atomic,
                        explicit_align: None,
                    };
                    return QualType::unqualified(Type::Pointer {
                        pointee: Box::new(pointee),
                    });
                }
                if are_compatible_unqualified(ap, bp, &ctx.type_ctx) {
                    // composite-ish — element qualifiers union
                    let pointee = QualType {
                        ty: ap.ty.clone(),
                        is_const: ap.is_const || bp.is_const,
                        is_volatile: ap.is_volatile || bp.is_volatile,
                        is_restrict: ap.is_restrict || bp.is_restrict,
                        is_atomic: ap.is_atomic || bp.is_atomic,
                        explicit_align: None,
                    };
                    return QualType::unqualified(Type::Pointer {
                        pointee: Box::new(pointee),
                    });
                }
            }
        }
    }

    ctx.emit(error("ternary operands have incompatible types", span));
    a_ty
}

// =========================================================================
// Compound literal
// =========================================================================

#[allow(clippy::too_many_arguments)]
fn check_compound_literal(
    tn: &TypeName,
    init: &Initializer,
    span: Span,
    node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let _ = span;
    let Some(ty) = resolve_type_name(tn, table, target, ctx) else {
        return QualType::unqualified(Type::Int { is_unsigned: false });
    };
    let refined = check_initializer(init, &ty, table, target, ctx);
    // Compound literals denote addressable objects — they are lvalues.
    ctx.mark_lvalue(node_id);
    refined
}

// =========================================================================
// _Generic
// =========================================================================

#[allow(clippy::too_many_arguments)]
fn check_generic(
    controlling: &Expr,
    associations: &[GenericAssociation],
    span: Span,
    _node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    // C17 §6.5.1.1p2-3 — the controlling expression is NOT evaluated;
    // its type is determined WITHOUT lvalue conversion, but WITH array-
    // to-pointer and function-to-pointer decay.  We approximate by
    // type-checking it in SizeofOperand context (no L2R, no decay), then
    // manually applying the two decays and stripping qualifiers.
    let raw = check_expr_in_context(controlling, ValueContext::SizeofOperand, table, target, ctx);
    let controller_ty = decay_for_generic(&raw);

    // Walk the associations.  Only one may be type-checked — the one we
    // select — because C17 says unselected arms are NOT type-checked.
    let mut default_arm: Option<&GenericAssociation> = None;
    let mut matches: Vec<&GenericAssociation> = Vec::new();
    for assoc in associations {
        match &assoc.type_name {
            None => {
                if default_arm.is_some() {
                    ctx.emit(error("duplicate default: in _Generic", span));
                }
                default_arm = Some(assoc);
            }
            Some(tn) => {
                if let Some(arm_ty) = resolve_type_name(tn, table, target, ctx) {
                    if are_compatible_unqualified(&controller_ty, &arm_ty, &ctx.type_ctx) {
                        matches.push(assoc);
                    }
                }
            }
        }
    }

    let selected = if matches.len() > 1 {
        ctx.emit(error(
            "more than one _Generic association matches the controlling type",
            span,
        ));
        matches[0]
    } else if matches.len() == 1 {
        matches[0]
    } else if let Some(def) = default_arm {
        def
    } else {
        ctx.emit(error(
            "no _Generic association matches the controlling type and there is no default:",
            span,
        ));
        return QualType::unqualified(Type::Int { is_unsigned: false });
    };

    // Only the selected expression is type-checked.
    check_expr_in_context(&selected.expr, ValueContext::RValue, table, target, ctx)
}

fn decay_for_generic(raw: &QualType) -> QualType {
    let ty = match &raw.ty {
        Type::Array { element, .. } => Type::Pointer {
            pointee: Box::new((**element).clone()),
        },
        Type::Function { .. } => Type::Pointer {
            pointee: Box::new(raw.clone()),
        },
        other => other.clone(),
    };
    QualType::unqualified(ty)
}

// =========================================================================
// Comma
// =========================================================================

fn check_comma(
    exprs: &[Expr],
    span: Span,
    _node_id: NodeId,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    if exprs.is_empty() {
        ctx.emit(error("empty comma expression", span));
        return QualType::unqualified(Type::Int { is_unsigned: false });
    }
    // Type-check every operand for side effects, but only the last one's
    // type escapes.
    let mut last = QualType::unqualified(Type::Int { is_unsigned: false });
    for (i, e) in exprs.iter().enumerate() {
        let qt = check_expr_in_context(e, ValueContext::RValue, table, target, ctx);
        if i == exprs.len() - 1 {
            last = qt;
        }
    }
    last
}

// =========================================================================
// Diagnostic helpers
// =========================================================================

fn error(msg: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::error(msg).span(span)
}

fn warning(msg: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::warning(msg).span(span)
}

// =========================================================================
// Node-id helper
// =========================================================================

fn expr_node_id(expr: &Expr) -> NodeId {
    match expr {
        Expr::IntLiteral { node_id, .. }
        | Expr::FloatLiteral { node_id, .. }
        | Expr::CharLiteral { node_id, .. }
        | Expr::StringLiteral { node_id, .. }
        | Expr::Ident { node_id, .. }
        | Expr::BinaryOp { node_id, .. }
        | Expr::UnaryOp { node_id, .. }
        | Expr::PostfixOp { node_id, .. }
        | Expr::Conditional { node_id, .. }
        | Expr::Assignment { node_id, .. }
        | Expr::FunctionCall { node_id, .. }
        | Expr::MemberAccess { node_id, .. }
        | Expr::ArraySubscript { node_id, .. }
        | Expr::Cast { node_id, .. }
        | Expr::SizeofExpr { node_id, .. }
        | Expr::SizeofType { node_id, .. }
        | Expr::AlignofType { node_id, .. }
        | Expr::CompoundLiteral { node_id, .. }
        | Expr::GenericSelection { node_id, .. }
        | Expr::Comma { node_id, .. }
        | Expr::BuiltinOffsetof { node_id, .. }
        | Expr::BuiltinTypesCompatibleP { node_id, .. } => *node_id,
    }
}
