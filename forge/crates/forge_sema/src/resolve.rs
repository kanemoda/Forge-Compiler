//! Resolve parser AST specifiers and declarators into sema types.
//!
//! This module owns two responsibilities:
//!
//! * [`resolve_type_specifiers`] turns a bag of primitive keywords,
//!   struct/union/enum specifiers, typedef references, type qualifiers,
//!   and an optional `_Alignas` into a single [`QualType`].
//!
//! * [`resolve_declarator`] walks a [`Declarator`] outside-in (pointers
//!   first, then the direct declarator), feeding each layer into
//!   [`Type`] so that by the time we reach the inner identifier the
//!   accumulated type matches what C would assign to it.
//!
//! Errors are pushed onto [`SemaContext`] and the offending function
//! returns [`None`].  The parser already established that the input is
//! grammatically valid C, so the errors here are all **semantic** (e.g.
//! `float double`, `_Alignas(1)` that weakens alignment).
//!
//! Every function in this module takes `table: &mut SymbolTable` rather
//! than an immutable borrow — resolving an `enum { A = 1 }` specifier
//! registers `A` as an ordinary-namespace symbol in the enclosing
//! scope, and that path must be available from every type-resolution
//! entry point (a cast, a `sizeof(T)`, a parameter type, etc.).

use forge_diagnostics::Diagnostic;
use forge_lexer::Span;
use forge_parser::ast::{
    AbstractDeclarator, AlignSpec, ArraySize as ParserArraySize, DeclSpecifiers, Declarator,
    DirectAbstractDeclarator, DirectDeclarator, Expr, ParamDecl, PointerQualifiers, StructDef,
    StructOrUnion, TypeName, TypeQualifier, TypeSpecifierToken,
};

use crate::context::SemaContext;
use crate::scope::{SymbolKind, SymbolTable};
use crate::types::{
    ArraySize, ParamType, QualType, Signedness, StructLayout, TargetInfo, Type, UnionLayout,
};

// =========================================================================
// Public entry points
// =========================================================================

/// Resolve a declaration's specifiers into the base [`QualType`] that
/// every declarator in the same declaration will start from.
pub fn resolve_type_specifiers(
    specifiers: &DeclSpecifiers,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<QualType> {
    let base = resolve_base_type(specifiers, table, target, ctx)?;
    let mut qt = QualType::unqualified(base);
    apply_qualifier_list(&mut qt, &specifiers.type_qualifiers);

    if let Some(align) = &specifiers.alignment {
        apply_align_spec(&mut qt, align, table, target, ctx)?;
    }

    Some(qt)
}

/// Walk `declarator`, returning the declared name (if any) and the
/// resulting [`QualType`].
///
/// * `base_type` is the [`QualType`] produced by [`resolve_type_specifiers`].
/// * `is_parameter` toggles the C17 §6.7.6.3 array-to-pointer adjustment
///   for function parameters (and carries the bracket qualifiers onto
///   the resulting pointer).
pub fn resolve_declarator(
    base_type: QualType,
    declarator: &Declarator,
    is_parameter: bool,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<(Option<String>, QualType)> {
    let mut t = base_type;
    for ptr in &declarator.pointers {
        t = wrap_pointer(t, ptr);
    }
    walk_direct(t, &declarator.direct, is_parameter, table, target, ctx)
}

/// Return the identifier buried inside a declarator, if any.
///
/// A declarator like `*(*fp)(int)` has one name; abstract declarators
/// (those used in `sizeof(int *)` or parameter declarations without a
/// name) have `None`.
pub fn declarator_name(decl: &Declarator) -> Option<String> {
    direct_name(&decl.direct)
}

// =========================================================================
// Base-type resolution
// =========================================================================

#[derive(Default)]
struct PrimitiveCounts {
    void: u32,
    bool_: u32,
    char: u32,
    short: u32,
    int: u32,
    long: u32,
    float: u32,
    double: u32,
    signed: u32,
    unsigned: u32,
    complex: u32,
}

fn resolve_base_type(
    specifiers: &DeclSpecifiers,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<Type> {
    let mut counts = PrimitiveCounts::default();
    let mut typedef_ty: Option<(QualType, Span)> = None;
    let mut tag_ty: Option<Type> = None;
    let mut atomic_ty: Option<QualType> = None;

    for tok in &specifiers.type_specifiers {
        match tok {
            TypeSpecifierToken::Void => counts.void += 1,
            TypeSpecifierToken::Bool => counts.bool_ += 1,
            TypeSpecifierToken::Char => counts.char += 1,
            TypeSpecifierToken::Short => counts.short += 1,
            TypeSpecifierToken::Int => counts.int += 1,
            TypeSpecifierToken::Long => counts.long += 1,
            TypeSpecifierToken::Float => counts.float += 1,
            TypeSpecifierToken::Double => counts.double += 1,
            TypeSpecifierToken::Signed => counts.signed += 1,
            TypeSpecifierToken::Unsigned => counts.unsigned += 1,
            TypeSpecifierToken::Complex => counts.complex += 1,
            TypeSpecifierToken::TypedefName(name) => {
                let sym = table.lookup(name);
                match sym {
                    Some(s) if matches!(s.kind, SymbolKind::Typedef) => {
                        if typedef_ty.is_some() {
                            ctx.emit(error(
                                "multiple typedef names in a single declaration",
                                specifiers.span,
                            ));
                            return None;
                        }
                        typedef_ty = Some((s.ty.clone(), specifiers.span));
                    }
                    _ => {
                        ctx.emit(error(
                            format!("'{name}' is not a typedef name"),
                            specifiers.span,
                        ));
                        return None;
                    }
                }
            }
            TypeSpecifierToken::Struct(def) => {
                if tag_ty.is_some() {
                    ctx.emit(error(
                        "multiple struct / union / enum specifiers",
                        specifiers.span,
                    ));
                    return None;
                }
                tag_ty = Some(resolve_struct_or_union(
                    def,
                    StructOrUnion::Struct,
                    table,
                    target,
                    ctx,
                )?);
            }
            TypeSpecifierToken::Union(def) => {
                if tag_ty.is_some() {
                    ctx.emit(error(
                        "multiple struct / union / enum specifiers",
                        specifiers.span,
                    ));
                    return None;
                }
                tag_ty = Some(resolve_struct_or_union(
                    def,
                    StructOrUnion::Union,
                    table,
                    target,
                    ctx,
                )?);
            }
            TypeSpecifierToken::Enum(def) => {
                if tag_ty.is_some() {
                    ctx.emit(error(
                        "multiple struct / union / enum specifiers",
                        specifiers.span,
                    ));
                    return None;
                }
                tag_ty = Some(resolve_enum(def, table, target, ctx)?);
            }
            TypeSpecifierToken::Atomic(inner) => {
                if atomic_ty.is_some() {
                    ctx.emit(error("multiple _Atomic(type) specifiers", specifiers.span));
                    return None;
                }
                let resolved = resolve_type_name(inner, table, target, ctx)?;
                atomic_ty = Some(resolved);
            }
            TypeSpecifierToken::TypeofExpr(inner) => {
                if typedef_ty.is_some() {
                    ctx.emit(error(
                        "__typeof__ combined with other type specifiers",
                        specifiers.span,
                    ));
                    return None;
                }
                // C / GCC `__typeof__(expr)`: evaluate the expression's
                // type without emitting the value; strip top-level
                // qualifiers the way GCC does for the type selected.
                let qt = crate::expr::check_expr(inner, table, target, ctx);
                typedef_ty = Some((qt, specifiers.span));
            }
            TypeSpecifierToken::TypeofType(tn) => {
                if typedef_ty.is_some() {
                    ctx.emit(error(
                        "__typeof__ combined with other type specifiers",
                        specifiers.span,
                    ));
                    return None;
                }
                let qt = resolve_type_name(tn, table, target, ctx)?;
                typedef_ty = Some((qt, specifiers.span));
            }
        }
    }

    // A typedef name is mutually exclusive with any primitive keyword
    // and any struct/union/enum or `_Atomic(type)`.
    if let Some((td_ty, _)) = typedef_ty {
        if has_any_primitive(&counts) || tag_ty.is_some() || atomic_ty.is_some() {
            ctx.emit(error(
                "typedef name combined with other type specifiers",
                specifiers.span,
            ));
            return None;
        }
        return Some(td_ty.ty);
    }

    if let Some(atomic) = atomic_ty {
        if has_any_primitive(&counts) || tag_ty.is_some() {
            ctx.emit(error(
                "_Atomic(type) combined with other type specifiers",
                specifiers.span,
            ));
            return None;
        }
        return Some(atomic.ty);
    }

    if let Some(tag) = tag_ty {
        if has_any_primitive(&counts) {
            ctx.emit(error(
                "struct / union / enum combined with primitive type keywords",
                specifiers.span,
            ));
            return None;
        }
        return Some(tag);
    }

    resolve_primitives(&counts, specifiers.span, ctx)
}

fn has_any_primitive(c: &PrimitiveCounts) -> bool {
    c.void
        + c.bool_
        + c.char
        + c.short
        + c.int
        + c.long
        + c.float
        + c.double
        + c.signed
        + c.unsigned
        + c.complex
        > 0
}

fn resolve_primitives(c: &PrimitiveCounts, span: Span, ctx: &mut SemaContext) -> Option<Type> {
    // Gross validation first — anything that duplicates a standalone
    // keyword or mixes mutually-exclusive groups is an error.
    if c.void > 1 || c.bool_ > 1 || c.char > 1 || c.short > 1 {
        ctx.emit(error("duplicate type specifier", span));
        return None;
    }
    if c.int > 1 {
        ctx.emit(error("duplicate 'int' specifier", span));
        return None;
    }
    if c.long > 2 {
        ctx.emit(error("too many 'long' specifiers", span));
        return None;
    }
    if c.signed > 1 || c.unsigned > 1 {
        ctx.emit(error("duplicate signedness specifier", span));
        return None;
    }
    if c.signed > 0 && c.unsigned > 0 {
        ctx.emit(error("'signed' and 'unsigned' cannot be combined", span));
        return None;
    }
    if c.float > 1 || c.double > 1 {
        ctx.emit(error("duplicate floating-point specifier", span));
        return None;
    }
    if c.float > 0 && c.double > 0 {
        ctx.emit(error("'float' and 'double' cannot be combined", span));
        return None;
    }

    // `void`: exclusive with everything.
    if c.void > 0 {
        if c.bool_
            + c.char
            + c.short
            + c.int
            + c.long
            + c.float
            + c.double
            + c.signed
            + c.unsigned
            + c.complex
            > 0
        {
            ctx.emit(error("'void' combined with other type specifiers", span));
            return None;
        }
        return Some(Type::Void);
    }

    // `_Bool`: exclusive with everything except (conceptually nothing).
    if c.bool_ > 0 {
        if c.char
            + c.short
            + c.int
            + c.long
            + c.float
            + c.double
            + c.signed
            + c.unsigned
            + c.complex
            > 0
        {
            ctx.emit(error("'_Bool' combined with other type specifiers", span));
            return None;
        }
        return Some(Type::Bool);
    }

    // Floating-point path.
    if c.float > 0 {
        if c.int + c.short + c.long + c.signed + c.unsigned + c.char > 0 {
            ctx.emit(error(
                "'float' cannot be combined with integer specifiers",
                span,
            ));
            return None;
        }
        if c.complex > 0 {
            ctx.emit(error("_Complex is not supported yet", span));
            return None;
        }
        return Some(Type::Float);
    }

    if c.double > 0 {
        if c.int + c.short + c.signed + c.unsigned + c.char > 0 {
            ctx.emit(error(
                "'double' cannot be combined with integer specifiers",
                span,
            ));
            return None;
        }
        if c.long > 1 {
            ctx.emit(error("too many 'long' specifiers for 'double'", span));
            return None;
        }
        if c.complex > 0 {
            ctx.emit(error("_Complex is not supported yet", span));
            return None;
        }
        return Some(if c.long == 1 {
            Type::LongDouble
        } else {
            Type::Double
        });
    }

    if c.complex > 0 {
        ctx.emit(error("_Complex is not supported yet", span));
        return None;
    }

    // Integer path.
    if c.char > 0 {
        if c.short + c.long + c.int > 0 {
            ctx.emit(error(
                "'char' cannot be combined with 'short', 'long', or 'int'",
                span,
            ));
            return None;
        }
        let signedness = if c.signed > 0 {
            Signedness::Signed
        } else if c.unsigned > 0 {
            Signedness::Unsigned
        } else {
            Signedness::Plain
        };
        return Some(Type::Char { signedness });
    }

    if c.short > 0 {
        if c.long > 0 {
            ctx.emit(error("'short' and 'long' cannot be combined", span));
            return None;
        }
        return Some(Type::Short {
            is_unsigned: c.unsigned > 0,
        });
    }

    if c.long == 2 {
        return Some(Type::LongLong {
            is_unsigned: c.unsigned > 0,
        });
    }

    if c.long == 1 {
        return Some(Type::Long {
            is_unsigned: c.unsigned > 0,
        });
    }

    // Bare `int`, bare `signed`, bare `unsigned`.
    if c.int + c.signed + c.unsigned > 0 {
        return Some(Type::Int {
            is_unsigned: c.unsigned > 0,
        });
    }

    ctx.emit(error("missing type specifier", span));
    None
}

// =========================================================================
// Struct / Union / Enum
// =========================================================================

fn resolve_struct_or_union(
    def: &StructDef,
    kind: StructOrUnion,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<Type> {
    let has_body = def.members.is_some();
    let name = def.name.clone();
    let span = def.span;

    match kind {
        StructOrUnion::Struct => {
            // Tag lookup: if the name is already bound in any enclosing
            // scope and the current declaration is not introducing a new
            // tag in the current scope, reuse the existing id so that
            // `struct P { ... };` and `struct P p;` share a layout.
            let existing = name
                .as_deref()
                .and_then(|n| table.lookup_tag(n))
                .and_then(|(_, e)| match e {
                    crate::scope::TagEntry::Struct(sid) => Some(*sid),
                    _ => None,
                });
            let sid = if let Some(sid) = existing {
                sid
            } else {
                let sid = ctx.type_ctx.fresh_struct_id();
                ctx.type_ctx.set_struct(
                    sid,
                    StructLayout {
                        tag: name.clone(),
                        ..StructLayout::default()
                    },
                );
                if let Some(n) = &name {
                    let _ = table.declare_tag(n, crate::scope::TagEntry::Struct(sid), span, ctx);
                }
                sid
            };
            if has_body {
                crate::layout::complete_struct(sid, def, table, target, ctx);
            }
            Some(Type::Struct(sid))
        }
        StructOrUnion::Union => {
            let existing = name
                .as_deref()
                .and_then(|n| table.lookup_tag(n))
                .and_then(|(_, e)| match e {
                    crate::scope::TagEntry::Union(uid) => Some(*uid),
                    _ => None,
                });
            let uid = if let Some(uid) = existing {
                uid
            } else {
                let uid = ctx.type_ctx.fresh_union_id();
                ctx.type_ctx.set_union(
                    uid,
                    UnionLayout {
                        tag: name.clone(),
                        ..UnionLayout::default()
                    },
                );
                if let Some(n) = &name {
                    let _ = table.declare_tag(n, crate::scope::TagEntry::Union(uid), span, ctx);
                }
                uid
            };
            if has_body {
                crate::layout::complete_union(uid, def, table, target, ctx);
            }
            Some(Type::Union(uid))
        }
    }
}

fn resolve_enum(
    def: &forge_parser::ast::EnumDef,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<Type> {
    let eid = ctx.type_ctx.fresh_enum_id();
    ctx.type_ctx.set_enum(
        eid,
        crate::types::EnumLayout {
            tag: def.name.clone(),
            ..crate::types::EnumLayout::default()
        },
    );
    if def.enumerators.is_some() {
        crate::layout::complete_enum(eid, def, table, target, ctx);
    }
    Some(Type::Enum(eid))
}

// =========================================================================
// Qualifier / alignment application
// =========================================================================

fn apply_qualifier_list(qt: &mut QualType, quals: &[TypeQualifier]) {
    for q in quals {
        match q {
            TypeQualifier::Const => qt.is_const = true,
            TypeQualifier::Volatile => qt.is_volatile = true,
            TypeQualifier::Restrict => qt.is_restrict = true,
            TypeQualifier::Atomic => qt.is_atomic = true,
        }
    }
}

fn apply_align_spec(
    qt: &mut QualType,
    align: &AlignSpec,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<()> {
    let requested = match align {
        AlignSpec::AlignAsExpr(e) => {
            let v = crate::const_eval::eval_icx_as_u64(e, table, target, ctx)?;
            if v == 0 {
                ctx.emit(error(
                    "_Alignas requires a positive alignment",
                    expr_span(e),
                ));
                return None;
            }
            v
        }
        AlignSpec::AlignAsType(tn) => {
            let resolved = resolve_type_name(tn, table, target, ctx)?;
            let Some(align) = resolved.ty.align_of(target, &ctx.type_ctx) else {
                ctx.emit(error(
                    "_Alignas(type) refers to a type with no alignment",
                    tn.span,
                ));
                return None;
            };
            align
        }
    };

    if !requested.is_power_of_two() {
        let span = match align {
            AlignSpec::AlignAsExpr(e) => expr_span(e),
            AlignSpec::AlignAsType(tn) => tn.span,
        };
        ctx.emit(error("_Alignas value must be a power of two", span));
        return None;
    }

    let natural = qt.ty.align_of(target, &ctx.type_ctx).unwrap_or(1);
    if requested < natural {
        let span = match align {
            AlignSpec::AlignAsExpr(e) => expr_span(e),
            AlignSpec::AlignAsType(tn) => tn.span,
        };
        ctx.emit(error(
            "_Alignas cannot weaken the natural alignment of the type",
            span,
        ));
        return None;
    }

    qt.explicit_align = Some(requested);
    Some(())
}

// =========================================================================
// Declarator walker
// =========================================================================

fn wrap_pointer(pointee: QualType, ptr: &PointerQualifiers) -> QualType {
    let mut qt = QualType::unqualified(Type::Pointer {
        pointee: Box::new(pointee),
    });
    apply_qualifier_list(&mut qt, &ptr.qualifiers);
    qt
}

fn walk_direct(
    current: QualType,
    direct: &DirectDeclarator,
    is_parameter: bool,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<(Option<String>, QualType)> {
    match direct {
        DirectDeclarator::Identifier(name, _) => Some((Some(name.clone()), current)),
        DirectDeclarator::Parenthesized(inner) => {
            resolve_declarator(current, inner, is_parameter, table, target, ctx)
        }
        DirectDeclarator::Array {
            base,
            size,
            qualifiers,
            is_static,
            span,
        } => {
            let wrapped = build_array_layer(
                current,
                size,
                qualifiers,
                *is_static,
                *span,
                is_parameter,
                table,
                target,
                ctx,
                /* outer = */ matches!(**base, DirectDeclarator::Identifier(..)),
            )?;
            walk_direct(wrapped, base, is_parameter, table, target, ctx)
        }
        DirectDeclarator::Function {
            base,
            params,
            is_variadic,
            span: _,
        } => {
            let resolved = build_function_layer(current, params, *is_variadic, table, target, ctx)?;
            walk_direct(resolved, base, is_parameter, table, target, ctx)
        }
    }
}

/// Build the next outer type layer for an array suffix.
///
/// When `is_parameter` *and* this is the outermost array layer (i.e.
/// the immediate base is the declared identifier), the array is
/// adjusted to a pointer per C17 §6.7.6.3.  Qualifiers inside the
/// brackets move to the resulting pointer; `static` is recorded via
/// the separate [`ParamType::has_static_size`] field (not a qualifier).
#[allow(clippy::too_many_arguments)]
fn build_array_layer(
    element: QualType,
    size: &ParserArraySize,
    qualifiers: &[TypeQualifier],
    is_static: bool,
    span: Span,
    is_parameter: bool,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
    is_outermost_array: bool,
) -> Option<QualType> {
    // C17 §6.7.2.1p18(b): a struct with a flexible array member cannot
    // appear as the element type of an array.  Applies in parameter
    // position too — the source still names `T[N]` even if it will
    // adjust to `T*` afterwards.
    if let Type::Struct(sid) = &element.ty {
        if ctx
            .type_ctx
            .struct_layout(*sid)
            .is_some_and(|s| s.has_flexible_array)
        {
            ctx.emit(error(
                "array element type has a flexible array member",
                span,
            ));
            return None;
        }
    }

    let array_size = match size {
        ParserArraySize::Unspecified => ArraySize::Incomplete,
        ParserArraySize::VLAStar => ArraySize::Star,
        ParserArraySize::Expr(e) => match eval_array_size(e, table, target, ctx) {
            Some(n) if n > 0 => ArraySize::Fixed(n as u64),
            Some(0) => ArraySize::Fixed(0),
            Some(_) => {
                ctx.emit(error("array size cannot be negative", expr_span(e)));
                return None;
            }
            None => ArraySize::Variable,
        },
    };

    if is_parameter && is_outermost_array {
        // Adjust T[N] → T*; quals in the brackets decorate the pointer.
        let mut ptr = QualType::unqualified(Type::Pointer {
            pointee: Box::new(element),
        });
        apply_qualifier_list(&mut ptr, qualifiers);
        // `static` is a hint carried on the ParamType, not on the pointer.
        // Likewise the evaluated size is discarded here.
        let _ = (is_static, array_size, span);
        return Some(ptr);
    }

    // A non-parameter array with bracket qualifiers is an error: the
    // `[const ...]` syntax is only legal on parameters.
    if !qualifiers.is_empty() || is_static {
        ctx.emit(error(
            "type qualifiers / 'static' are only allowed inside the brackets of a function parameter",
            span,
        ));
        return None;
    }

    Some(QualType::unqualified(Type::Array {
        element: Box::new(element),
        size: array_size,
    }))
}

fn build_function_layer(
    return_type: QualType,
    params: &[ParamDecl],
    is_variadic: bool,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<QualType> {
    // An empty parameter list (`int f()`) is NOT a prototype in C17.
    // `int f(void)` IS a prototype with zero parameters.
    let empty_param_list = params.is_empty();
    let is_void_proto = matches!(
        params,
        [
            ParamDecl {
                specifiers,
                declarator,
                abstract_declarator,
                ..
            },
        ] if declarator.is_none()
            && abstract_declarator.is_none()
            && specifiers.type_specifiers.iter().any(|t| matches!(t, TypeSpecifierToken::Void))
            && specifiers.type_specifiers.len() == 1
            && specifiers.storage_class.is_none()
            && specifiers.type_qualifiers.is_empty()
    );

    let is_prototype = !empty_param_list;
    let resolved_params = if is_void_proto || empty_param_list {
        Vec::new()
    } else {
        let mut out = Vec::with_capacity(params.len());
        for p in params {
            let p_ty = resolve_param_decl(p, table, target, ctx)?;
            out.push(p_ty);
        }
        out
    };

    Some(QualType::unqualified(Type::Function {
        return_type: Box::new(return_type),
        params: resolved_params,
        is_variadic,
        is_prototype,
    }))
}

fn resolve_param_decl(
    param: &ParamDecl,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<ParamType> {
    let base = resolve_type_specifiers(&param.specifiers, table, target, ctx)?;

    // Helper: after the full type is built, detect whether the
    // outermost *array layer* was adjusted to a pointer and the source
    // contained `static` inside the brackets.
    let has_static_size = outermost_array_has_static(param);

    let (name, ty) = match (&param.declarator, &param.abstract_declarator) {
        (Some(decl), _) => resolve_declarator(
            base, decl, /* is_parameter = */ true, table, target, ctx,
        )?,
        (None, Some(abs)) => (
            None,
            resolve_abstract_declarator(base, abs, table, target, ctx)?,
        ),
        (None, None) => (None, base),
    };

    // C17 §6.7.6.3: a parameter of type function-returning-T adjusts to
    // a pointer-to-function-returning-T.
    let adjusted = adjust_function_to_pointer(ty);

    Some(ParamType {
        name,
        ty: adjusted,
        has_static_size,
    })
}

fn adjust_function_to_pointer(ty: QualType) -> QualType {
    if matches!(ty.ty, Type::Function { .. }) {
        QualType::unqualified(Type::Pointer {
            pointee: Box::new(ty),
        })
    } else {
        ty
    }
}

fn outermost_array_has_static(param: &ParamDecl) -> bool {
    let Some(decl) = &param.declarator else {
        return false;
    };
    if !decl.pointers.is_empty() {
        return false;
    }
    match &decl.direct {
        DirectDeclarator::Array {
            base, is_static, ..
        } => *is_static && matches!(**base, DirectDeclarator::Identifier(..)),
        _ => false,
    }
}

fn direct_name(direct: &DirectDeclarator) -> Option<String> {
    match direct {
        DirectDeclarator::Identifier(name, _) => Some(name.clone()),
        DirectDeclarator::Parenthesized(inner) => declarator_name(inner),
        DirectDeclarator::Array { base, .. } | DirectDeclarator::Function { base, .. } => {
            direct_name(base)
        }
    }
}

// =========================================================================
// TypeName resolution (used by _Atomic(TN), _Alignas(TN), casts, etc.)
// =========================================================================

pub(crate) fn resolve_type_name(
    tn: &TypeName,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<QualType> {
    let base = resolve_type_specifiers(&tn.specifiers, table, target, ctx)?;
    match &tn.abstract_declarator {
        None => Some(base),
        Some(abs) => resolve_abstract_declarator(base, abs, table, target, ctx),
    }
}

fn resolve_abstract_declarator(
    base: QualType,
    abs: &AbstractDeclarator,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<QualType> {
    let mut t = base;
    for ptr in &abs.pointers {
        t = wrap_pointer(t, ptr);
    }
    match &abs.direct {
        None => Some(t),
        Some(dir) => resolve_direct_abstract(t, dir, table, target, ctx),
    }
}

fn resolve_direct_abstract(
    current: QualType,
    direct: &DirectAbstractDeclarator,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<QualType> {
    match direct {
        DirectAbstractDeclarator::Parenthesized(inner) => {
            resolve_abstract_declarator(current, inner, table, target, ctx)
        }
        DirectAbstractDeclarator::Array { base, size, span } => {
            let wrapped = build_array_layer(
                current,
                size,
                &[],
                false,
                *span,
                /* is_parameter = */ false,
                table,
                target,
                ctx,
                /* outermost = */ base.is_none(),
            )?;
            match base {
                None => Some(wrapped),
                Some(b) => resolve_direct_abstract(wrapped, b, table, target, ctx),
            }
        }
        DirectAbstractDeclarator::Function {
            base,
            params,
            is_variadic,
            span: _,
        } => {
            let wrapped = build_function_layer(current, params, *is_variadic, table, target, ctx)?;
            match base {
                None => Some(wrapped),
                Some(b) => resolve_direct_abstract(wrapped, b, table, target, ctx),
            }
        }
    }
}

// =========================================================================
// Constant-expression helpers
// =========================================================================

/// Evaluate an array-size expression as an integer constant, falling
/// back to `None` (VLA) when the expression is not a compile-time
/// constant.  Diagnostics emitted by the full constant-evaluator while
/// probing are swallowed: a runtime size is legal at this position and
/// should not surface as a hard error.
fn eval_array_size(
    expr: &Expr,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> Option<i64> {
    let saved = ctx.diagnostics.len();
    let result = crate::const_eval::eval_icx_as_i64(expr, table, target, ctx);
    if result.is_none() {
        ctx.diagnostics.truncate(saved);
    }
    result
}

pub(crate) fn expr_span(expr: &Expr) -> Span {
    match expr {
        Expr::IntLiteral { span, .. }
        | Expr::FloatLiteral { span, .. }
        | Expr::CharLiteral { span, .. }
        | Expr::StringLiteral { span, .. }
        | Expr::Ident { span, .. }
        | Expr::BinaryOp { span, .. }
        | Expr::UnaryOp { span, .. }
        | Expr::PostfixOp { span, .. }
        | Expr::Conditional { span, .. }
        | Expr::Assignment { span, .. }
        | Expr::FunctionCall { span, .. }
        | Expr::MemberAccess { span, .. }
        | Expr::ArraySubscript { span, .. }
        | Expr::Cast { span, .. }
        | Expr::SizeofExpr { span, .. }
        | Expr::SizeofType { span, .. }
        | Expr::AlignofType { span, .. }
        | Expr::CompoundLiteral { span, .. }
        | Expr::GenericSelection { span, .. }
        | Expr::Comma { span, .. }
        | Expr::BuiltinOffsetof { span, .. }
        | Expr::BuiltinTypesCompatibleP { span, .. } => *span,
    }
}

// =========================================================================
// Diagnostic helper
// =========================================================================

fn error(msg: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::error(msg).span(span)
}
