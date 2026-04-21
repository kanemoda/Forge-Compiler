//! Analyse declarations and their initialisers.
//!
//! This module owns the third phase of sema: given a parser [`Declaration`],
//! turn each init-declarator into a [`Symbol`] with the right linkage,
//! storage class, function-specifier flags, and `is_defined` flag, and
//! register it in the current scope.
//!
//! The work splits into two entry points:
//!
//! * [`analyze_declaration`] walks a single [`Declaration`] — resolves
//!   specifiers once, then for each [`InitDeclarator`] resolves the
//!   declarator, computes linkage, checks any initialiser, and declares
//!   the symbol.
//! * [`check_initializer`] validates that an [`Initializer`] fits the
//!   target [`QualType`].  Arrays, structs, and unions accept
//!   brace-enclosed lists (with designators); scalars accept a single
//!   expression.  String-literal initialisation of `char` arrays is
//!   recognised as a special case of scalar initialiser.
//!
//! Linkage and storage-class handling follows C17 §6.2.2.  File-scope
//! declarations without an initialiser are *tentative definitions* per
//! §6.9.2 and are merged by [`SymbolTable::declare`] with any later
//! definition of the same name.
//!
//! TODO(phase5): `inline` at file scope interacts with linkage in a way
//! that requires tracking "inline definition" vs "external definition"
//! separately — deferred here until link-time semantics land.

use forge_diagnostics::Diagnostic;
use forge_lexer::Span;
use forge_parser::ast::{
    DeclSpecifiers, Declaration, DesignatedInit, Designator, DirectDeclarator, Expr,
    FunctionSpecifier, InitDeclarator, Initializer, StaticAssert,
    StorageClass as ParserStorageClass,
};

use crate::const_eval::eval_icx_as_i64;
use crate::context::SemaContext;
use crate::resolve::{expr_span, resolve_declarator, resolve_type_specifiers};
use crate::scope::{Linkage, ScopeKind, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{ArraySize, QualType, StructTypeId, TargetInfo, Type, UnionTypeId};

// =========================================================================
// Public entry points
// =========================================================================

/// Analyse a top-level or block-scope declaration.
///
/// Resolves the shared [`DeclSpecifiers`], then walks each
/// [`InitDeclarator`]: builds the declarator's type, derives the
/// storage class / linkage / function-specifier flags, checks any
/// initialiser, and registers a [`Symbol`] in the current scope.
pub fn analyze_declaration(
    decl: &Declaration,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    let Some(base_ty) = resolve_type_specifiers(&decl.specifiers, table, target, ctx) else {
        return;
    };

    // A declaration with no declarators is legal when the specifiers
    // themselves introduced a tag (`struct foo { int x; };` or
    // `enum E { A, B };`).  There is nothing further to bind.
    if decl.init_declarators.is_empty() {
        return;
    }

    let is_typedef = matches!(
        decl.specifiers.storage_class,
        Some(ParserStorageClass::Typedef)
    );
    let storage = convert_storage(decl.specifiers.storage_class);
    let (is_inline, is_noreturn) =
        extract_function_specifiers(&decl.specifiers.function_specifiers);

    for init_decl in &decl.init_declarators {
        if is_typedef {
            analyze_typedef_declarator(
                init_decl,
                base_ty.clone(),
                &decl.specifiers,
                table,
                target,
                ctx,
            );
        } else {
            analyze_object_or_function_declarator(
                init_decl,
                base_ty.clone(),
                storage,
                is_inline,
                is_noreturn,
                table,
                target,
                ctx,
            );
        }
    }
}

/// Analyse a file-scope or block-scope `_Static_assert`.
pub fn analyze_static_assert(
    sa: &StaticAssert,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    let Some(cond) = eval_icx_as_i64(&sa.condition, table, target, ctx) else {
        return;
    };
    if cond == 0 {
        let msg = sa
            .message
            .clone()
            .unwrap_or_else(|| "static assertion failed".into());
        ctx.emit(Diagnostic::error(msg).span(sa.span));
    }
}

/// Verify that `init` is a valid initialiser for an object of type
/// `target_ty`.
///
/// Returns the (possibly refined) target type — an incomplete array
/// target may be widened to a fixed size based on the initialiser's
/// element count or string-literal byte count.  Emits diagnostics on
/// structural mismatch but always returns a usable type so the caller
/// can still register the symbol.
pub fn check_initializer(
    init: &Initializer,
    target_ty: &QualType,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    match init {
        Initializer::Expr(expr) => check_scalar_or_string_init(expr, target_ty, table, target, ctx),
        Initializer::List { items, span, .. } => {
            check_brace_init(items, target_ty, *span, table, target, ctx)
        }
    }
}

// =========================================================================
// Storage class / function specifier conversion
// =========================================================================

fn convert_storage(sc: Option<ParserStorageClass>) -> StorageClass {
    match sc {
        None => StorageClass::None,
        Some(ParserStorageClass::Auto) => StorageClass::Auto,
        Some(ParserStorageClass::Register) => StorageClass::Register,
        Some(ParserStorageClass::Static) => StorageClass::Static,
        Some(ParserStorageClass::Extern) => StorageClass::Extern,
        Some(ParserStorageClass::ThreadLocal) => StorageClass::ThreadLocal,
        // Caller checks for Typedef up front via a separate path; if it
        // somehow gets here we fall through to the default (never a
        // legal object/function storage class).
        Some(ParserStorageClass::Typedef) => StorageClass::None,
    }
}

fn extract_function_specifiers(specs: &[FunctionSpecifier]) -> (bool, bool) {
    let mut is_inline = false;
    let mut is_noreturn = false;
    for s in specs {
        match s {
            FunctionSpecifier::Inline => is_inline = true,
            FunctionSpecifier::Noreturn => is_noreturn = true,
        }
    }
    (is_inline, is_noreturn)
}

// =========================================================================
// Typedef path
// =========================================================================

fn analyze_typedef_declarator(
    init_decl: &InitDeclarator,
    base_ty: QualType,
    specifiers: &DeclSpecifiers,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    if init_decl.initializer.is_some() {
        ctx.emit(
            Diagnostic::error("typedef declaration cannot have an initializer")
                .span(init_decl.span),
        );
    }

    let Some((Some(name), ty)) =
        resolve_declarator(base_ty, &init_decl.declarator, false, table, target, ctx)
    else {
        return;
    };

    // typedefs pick up `inline`/`_Noreturn` only cosmetically — they do
    // not apply, but we do not reject silly specifier combinations
    // either.
    let _ = specifiers;

    let sym = Symbol {
        id: 0,
        name,
        ty,
        kind: SymbolKind::Typedef,
        storage: StorageClass::None,
        linkage: Linkage::None,
        span: declarator_ident_span(init_decl).unwrap_or(init_decl.span),
        is_defined: true,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
    };
    let _ = table.declare(sym, ctx);
}

// =========================================================================
// Object / function path
// =========================================================================

#[allow(clippy::too_many_arguments)]
fn analyze_object_or_function_declarator(
    init_decl: &InitDeclarator,
    base_ty: QualType,
    storage: StorageClass,
    is_inline: bool,
    is_noreturn: bool,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    let Some((name, mut ty)) =
        resolve_declarator(base_ty, &init_decl.declarator, false, table, target, ctx)
    else {
        return;
    };
    let Some(name) = name else {
        ctx.emit(Diagnostic::error("declarator must have an identifier").span(init_decl.span));
        return;
    };

    let is_function = ty.ty.is_function();

    if init_decl.initializer.is_some() && is_function {
        ctx.emit(
            Diagnostic::error("function declaration cannot have an initializer")
                .span(init_decl.span),
        );
    }

    if let Some(init) = &init_decl.initializer {
        if !is_function {
            ty = check_initializer(init, &ty, table, target, ctx);
        }
    }

    let scope_kind = table.current_scope_kind();
    let linkage = determine_linkage(scope_kind, storage, is_function);
    let kind = if is_function {
        SymbolKind::Function
    } else {
        SymbolKind::Variable
    };

    let is_defined = symbol_is_defined(
        scope_kind,
        storage,
        is_function,
        init_decl.initializer.is_some(),
    );

    let sym = Symbol {
        id: 0,
        name,
        ty,
        kind,
        storage,
        linkage,
        span: declarator_ident_span(init_decl).unwrap_or(init_decl.span),
        is_defined,
        is_inline: is_inline && is_function,
        is_noreturn: is_noreturn && is_function,
        has_noreturn_attr: false,
    };

    let _ = table.declare(sym, ctx);
}

fn declarator_ident_span(init_decl: &InitDeclarator) -> Option<Span> {
    fn walk(d: &DirectDeclarator) -> Option<Span> {
        match d {
            DirectDeclarator::Identifier(_, span) => Some(*span),
            DirectDeclarator::Parenthesized(inner) => walk(&inner.direct),
            DirectDeclarator::Array { base, .. } | DirectDeclarator::Function { base, .. } => {
                walk(base)
            }
        }
    }
    walk(&init_decl.declarator.direct)
}

// =========================================================================
// Linkage (§6.2.2) and definition status (§6.9.2)
// =========================================================================

fn determine_linkage(scope: ScopeKind, storage: StorageClass, is_function: bool) -> Linkage {
    match scope {
        ScopeKind::File => match storage {
            StorageClass::Static => Linkage::Internal,
            _ => Linkage::External,
        },
        ScopeKind::Block | ScopeKind::Function => match storage {
            StorageClass::Extern => Linkage::External,
            _ => {
                // A block-scope function declaration without an
                // explicit storage class has external linkage per
                // §6.2.2p5.
                if is_function {
                    Linkage::External
                } else {
                    Linkage::None
                }
            }
        },
        ScopeKind::Prototype => Linkage::None,
    }
}

fn symbol_is_defined(
    scope: ScopeKind,
    storage: StorageClass,
    is_function: bool,
    has_initializer: bool,
) -> bool {
    if is_function {
        // A bare function declaration is never a definition; the
        // function-definition syntax (with a body) is handled outside
        // this module.
        return false;
    }
    if has_initializer {
        return true;
    }
    match scope {
        ScopeKind::Block | ScopeKind::Function => !matches!(storage, StorageClass::Extern),
        ScopeKind::File => {
            // File-scope objects without an initialiser are *tentative*
            // definitions (§6.9.2).  We mark them as "not yet defined"
            // so a later non-tentative definition merges cleanly; the
            // driver may promote tentatives at end-of-TU.
            false
        }
        ScopeKind::Prototype => false,
    }
}

// =========================================================================
// Initialiser checking — scalar / string
// =========================================================================

fn check_scalar_or_string_init(
    expr: &Expr,
    target_ty: &QualType,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    // Special case: `char arr[N] = "..."` / `char arr[] = "..."`.
    if let Expr::StringLiteral { value, .. } = expr {
        if let Type::Array { element, size } = &target_ty.ty {
            if matches!(element.ty, Type::Char { .. }) {
                return refine_char_array_from_string(target_ty, size, value.as_str());
            }
        }
    }

    match &target_ty.ty {
        Type::Array { .. } => {
            ctx.emit(
                Diagnostic::error("array initializer must be a brace-enclosed list")
                    .span(expr_span(expr)),
            );
        }
        Type::Struct(_) | Type::Union(_) => {
            ctx.emit(
                Diagnostic::error("struct or union initializer must be a brace-enclosed list")
                    .span(expr_span(expr)),
            );
        }
        Type::Function { .. } => {
            ctx.emit(Diagnostic::error("cannot initialize a function").span(expr_span(expr)));
        }
        Type::Void => {
            ctx.emit(
                Diagnostic::error("cannot initialize a value of type 'void'").span(expr_span(expr)),
            );
        }
        _ => {
            let rhs_ty = crate::expr::check_expr(expr, table, target, ctx);
            crate::expr::check_assignability(target_ty, &rhs_ty, expr, expr_span(expr), ctx);
        }
    }

    target_ty.clone()
}

fn refine_char_array_from_string(
    target_ty: &QualType,
    size: &ArraySize,
    literal: &str,
) -> QualType {
    let nbytes = literal.len() as u64;
    match size {
        ArraySize::Incomplete => {
            let mut refined = target_ty.clone();
            if let Type::Array { element, .. } = &target_ty.ty {
                refined.ty = Type::Array {
                    element: element.clone(),
                    size: ArraySize::Fixed(nbytes + 1),
                };
            }
            refined
        }
        _ => target_ty.clone(),
    }
}

// =========================================================================
// Initialiser checking — brace-enclosed
// =========================================================================

fn check_brace_init(
    items: &[DesignatedInit],
    target_ty: &QualType,
    span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    match &target_ty.ty {
        Type::Array { element, size } => {
            check_array_brace(items, element, size, target_ty, table, target, ctx)
        }
        Type::Struct(id) => check_struct_brace(items, *id, target_ty, span, table, target, ctx),
        Type::Union(id) => check_union_brace(items, *id, target_ty, span, table, target, ctx),
        _ if target_ty.ty.is_scalar() => {
            check_scalar_brace(items, span, ctx);
            target_ty.clone()
        }
        _ => {
            ctx.emit(
                Diagnostic::error("brace-enclosed initializer used with non-aggregate type")
                    .span(span),
            );
            target_ty.clone()
        }
    }
}

fn check_scalar_brace(items: &[DesignatedInit], span: Span, ctx: &mut SemaContext) {
    if items.is_empty() {
        return;
    }
    if items.len() > 1 {
        ctx.emit(Diagnostic::warning("excess elements in scalar initializer").span(span));
    }
    if !items[0].designators.is_empty() {
        ctx.emit(Diagnostic::error("designator used with scalar initializer").span(items[0].span));
    }
}

#[allow(clippy::too_many_arguments)]
fn check_array_brace(
    items: &[DesignatedInit],
    element: &QualType,
    size: &ArraySize,
    target_ty: &QualType,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let declared_size = match size {
        ArraySize::Fixed(n) => Some(*n),
        _ => None,
    };

    let mut current_index: u64 = 0;
    let mut highest_index: u64 = 0;
    let mut saw_any = false;

    for item in items {
        if let Some(first) = item.designators.first() {
            match first {
                Designator::Index(idx_expr) => {
                    if let Some(v) = eval_icx_as_i64(idx_expr, table, target, ctx) {
                        if v < 0 {
                            ctx.emit(
                                Diagnostic::error("array designator cannot be negative")
                                    .span(item.span),
                            );
                        } else {
                            current_index = v as u64;
                        }
                    }
                }
                Designator::Field(_) => {
                    ctx.emit(
                        Diagnostic::error("field designator used to initialize an array")
                            .span(item.span),
                    );
                }
            }
        }

        let _ = check_initializer(&item.initializer, element, table, target, ctx);

        saw_any = true;
        highest_index = highest_index.max(current_index);

        if let Some(n) = declared_size {
            if current_index >= n {
                ctx.emit(
                    Diagnostic::warning("excess elements in array initializer").span(item.span),
                );
            }
        }
        current_index += 1;
    }

    if matches!(size, ArraySize::Incomplete) && saw_any {
        let mut refined = target_ty.clone();
        refined.ty = Type::Array {
            element: Box::new(element.clone()),
            size: ArraySize::Fixed(highest_index + 1),
        };
        return refined;
    }

    target_ty.clone()
}

fn check_struct_brace(
    items: &[DesignatedInit],
    sid: StructTypeId,
    target_ty: &QualType,
    list_span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    let member_count = ctx
        .type_ctx
        .struct_layout(sid)
        .map(|s| s.members.len())
        .unwrap_or(0);

    if member_count > 0 && items.len() > member_count {
        ctx.emit(Diagnostic::warning("excess elements in struct initializer").span(list_span));
    }

    for item in items {
        if let Some(Designator::Index(_)) = item.designators.first() {
            ctx.emit(
                Diagnostic::error("array designator used to initialize a struct").span(item.span),
            );
        }
        // Recursively check without a per-field target type — expression
        // analysis will re-visit each initialiser and compare to the
        // matching member once field-designator tracking lands.  For
        // now we at least recurse into nested braces.
        if let Initializer::List { .. } = &*item.initializer {
            let _ = check_initializer(&item.initializer, target_ty, table, target, ctx);
        }
    }
    target_ty.clone()
}

fn check_union_brace(
    items: &[DesignatedInit],
    _uid: UnionTypeId,
    target_ty: &QualType,
    list_span: Span,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) -> QualType {
    if items.len() > 1 {
        let all_designated = items.iter().all(|it| !it.designators.is_empty());
        if !all_designated {
            ctx.emit(Diagnostic::warning("excess elements in union initializer").span(list_span));
        }
    }
    for item in items {
        if let Some(Designator::Index(_)) = item.designators.first() {
            ctx.emit(
                Diagnostic::error("array designator used to initialize a union").span(item.span),
            );
        }
        if let Initializer::List { .. } = &*item.initializer {
            let _ = check_initializer(&item.initializer, target_ty, table, target, ctx);
        }
    }
    target_ty.clone()
}
