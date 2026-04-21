//! Statement type checking and function-body analysis.
//!
//! Where [`crate::declare`] handled single [`Declaration`] nodes and
//! [`crate::expr`] handled single [`Expr`] nodes, this module walks the
//! statement-shaped part of the tree:
//!
//! * [`analyze_stmt`] — recursive walker for every [`Stmt`] variant.  It
//!   is driven by a borrowed [`FnContext`] that tracks per-function state
//!   (return type, loop / switch nesting, labels defined and referenced).
//! * [`analyze_function_def`] — entry point for a whole function
//!   definition: resolves the declarator, registers the function symbol,
//!   pushes the function scope, declares parameters, walks the body, and
//!   finally validates any labels referenced by `goto`.
//!
//! The walker is deliberately light on type inference — the heavy lifting
//! is delegated to [`check_expr`] for every expression context
//! (conditions, controlling expressions, case values, return values).
//! What this module *does* enforce is the set of structural rules that
//! only make sense on statements:
//!
//! * `if` / `while` / `for` / `do` conditions must be scalar;
//! * `break` must sit inside a loop or switch, `continue` inside a loop
//!   only (C17 §6.8.6.2: switch alone does not satisfy `continue`);
//! * `case` / `default` are only legal inside `switch`, case values must
//!   be integer constant expressions and must be pairwise distinct,
//!   and there can be at most one `default`;
//! * `return <expr>` requires a non-`void` return type; `return;`
//!   requires `void`;
//! * labels are function-scoped: a duplicate label is an error, a `goto`
//!   that never sees a matching label is an error.
//!
//! # Function body scope
//!
//! C17 §6.2.1p4 says the body of a function definition shares its
//! function scope with the parameters — the outermost compound
//! statement is *not* a new block.  Practically this means
//! [`analyze_function_def`] does not call `push_scope(Block)` before
//! walking the body; it iterates [`CompoundStmt::items`] directly.
//! Nested compounds inside the body do push a fresh block scope via
//! [`analyze_stmt`] / [`analyze_compound`].

use forge_diagnostics::Diagnostic;
use forge_lexer::Span;
use forge_parser::ast::{
    BlockItem, CompoundStmt, DirectDeclarator, Expr, ForInit, FunctionDef, ParamDecl, Stmt,
    StorageClass as ParserStorageClass,
};
use forge_parser::ast_ops::AssignOp;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::const_eval::eval_icx_as_i64;
use crate::context::SemaContext;
use crate::declare::{analyze_declaration, analyze_static_assert};
use crate::expr::check_expr;
use crate::resolve::resolve_declarator;
use crate::resolve::resolve_type_specifiers;
use crate::scope::{Linkage, ScopeKind, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{ParamType, QualType, TargetInfo, Type};

// =========================================================================
// FnContext
// =========================================================================

/// Information the sema walker needs to thread through a single function
/// body.  One [`FnContext`] is built per function definition, owned by
/// [`analyze_function_def`], and passed mutably to [`analyze_stmt`].
///
/// `in_loop` / `in_switch` form a stack of *currently-open* nesting
/// levels: we save the previous value on entry, overwrite it, recurse,
/// and restore on exit.  `switch_stack` is an explicit stack because a
/// `case` label's visibility depends on the innermost enclosing switch.
#[derive(Debug)]
pub struct FnContext {
    /// Return type of the function being analysed.
    pub return_type: QualType,
    /// `true` while the walker is inside the body of a `while` / `do`
    /// / `for` loop.
    pub in_loop: bool,
    /// `true` while the walker is inside the body of a `switch`.
    pub in_switch: bool,
    /// Stack of open switch statements, innermost last.
    pub switch_stack: Vec<SwitchInfo>,
    /// Labels defined by `label:` inside the function, with their span.
    pub labels_defined: FxHashMap<String, Span>,
    /// Labels referenced by `goto` — resolved at end of function.
    pub labels_referenced: Vec<(String, Span)>,
    /// `true` once at least one `return` statement has been seen.  Used
    /// for the non-void-returns warning heuristic.
    pub return_seen: bool,
    /// `true` if the function was declared `_Noreturn`.
    pub is_noreturn: bool,
}

impl FnContext {
    fn new(return_type: QualType, is_noreturn: bool) -> Self {
        Self {
            return_type,
            in_loop: false,
            in_switch: false,
            switch_stack: Vec::new(),
            labels_defined: FxHashMap::default(),
            labels_referenced: Vec::new(),
            return_seen: false,
            is_noreturn,
        }
    }
}

/// Per-switch state pushed on entry to a `switch` body and popped on
/// exit.  Tracks the set of seen case values and whether a `default`
/// has been seen, both used for duplicate detection.
#[derive(Debug, Default)]
pub struct SwitchInfo {
    /// Case values already seen in the innermost switch.  `FxHashSet`
    /// so duplicate detection is O(1).
    pub cases_seen: FxHashSet<i64>,
    /// `true` once a `default:` has appeared in the innermost switch.
    pub has_default: bool,
}

// =========================================================================
// analyze_stmt
// =========================================================================

/// Analyse one statement, mutating `fn_ctx`, `table`, and `ctx` as
/// needed.  See the module-level docs for the rules it enforces.
pub fn analyze_stmt(
    stmt: &Stmt,
    fn_ctx: &mut FnContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    match stmt {
        Stmt::Compound(block) => analyze_compound(block, fn_ctx, table, target, ctx),
        Stmt::Expr { expr, .. } => {
            if let Some(e) = expr {
                let _ = check_expr(e, table, target, ctx);
            }
        }
        Stmt::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => analyze_if(
            condition,
            then_branch,
            else_branch.as_deref(),
            fn_ctx,
            table,
            target,
            ctx,
        ),
        Stmt::While {
            condition, body, ..
        } => analyze_while(condition, body, fn_ctx, table, target, ctx),
        Stmt::DoWhile {
            body, condition, ..
        } => analyze_do_while(body, condition, fn_ctx, table, target, ctx),
        Stmt::For {
            init,
            condition,
            update,
            body,
            ..
        } => analyze_for(
            init,
            condition.as_deref(),
            update.as_deref(),
            body,
            fn_ctx,
            table,
            target,
            ctx,
        ),
        Stmt::Switch { expr, body, .. } => {
            analyze_switch(expr, body, fn_ctx, table, target, ctx);
        }
        Stmt::Case {
            value, body, span, ..
        } => analyze_case(value, body, *span, fn_ctx, table, target, ctx),
        Stmt::Default { body, span, .. } => {
            analyze_default(body, *span, fn_ctx, table, target, ctx)
        }
        Stmt::Return { value, span, .. } => {
            analyze_return(value.as_deref(), *span, fn_ctx, table, target, ctx)
        }
        Stmt::Break { span, .. } => analyze_break(*span, fn_ctx, ctx),
        Stmt::Continue { span, .. } => analyze_continue(*span, fn_ctx, ctx),
        Stmt::Goto { label, span, .. } => {
            fn_ctx.labels_referenced.push((label.clone(), *span));
        }
        Stmt::Label {
            name, stmt, span, ..
        } => analyze_label(name, stmt, *span, fn_ctx, table, target, ctx),
    }
}

// =========================================================================
// Compound and block items
// =========================================================================

fn analyze_compound(
    block: &CompoundStmt,
    fn_ctx: &mut FnContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    table.push_scope(ScopeKind::Block);
    for item in &block.items {
        analyze_block_item(item, fn_ctx, table, target, ctx);
    }
    table.pop_scope();
}

fn analyze_block_item(
    item: &BlockItem,
    fn_ctx: &mut FnContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    match item {
        BlockItem::Declaration(d) => analyze_declaration(d, table, target, ctx),
        BlockItem::Statement(s) => analyze_stmt(s, fn_ctx, table, target, ctx),
        BlockItem::StaticAssert(sa) => analyze_static_assert(sa, table, target, ctx),
    }
}

// =========================================================================
// If / While / DoWhile / For
// =========================================================================

fn analyze_if(
    condition: &Expr,
    then_branch: &Stmt,
    else_branch: Option<&Stmt>,
    fn_ctx: &mut FnContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    // The `if (x = 5)` typo warning fires before we check the condition
    // type so the warning survives even if the assignment doesn't
    // type-check cleanly.
    warn_on_assignment_in_condition(condition, ctx);
    let cond_ty = check_expr(condition, table, target, ctx);
    require_scalar_condition(&cond_ty, expr_span_of(condition), "if", ctx);
    analyze_stmt(then_branch, fn_ctx, table, target, ctx);
    if let Some(else_s) = else_branch {
        analyze_stmt(else_s, fn_ctx, table, target, ctx);
    }
}

fn analyze_while(
    condition: &Expr,
    body: &Stmt,
    fn_ctx: &mut FnContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    let cond_ty = check_expr(condition, table, target, ctx);
    require_scalar_condition(&cond_ty, expr_span_of(condition), "while", ctx);
    let saved = fn_ctx.in_loop;
    fn_ctx.in_loop = true;
    analyze_stmt(body, fn_ctx, table, target, ctx);
    fn_ctx.in_loop = saved;
}

fn analyze_do_while(
    body: &Stmt,
    condition: &Expr,
    fn_ctx: &mut FnContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    let saved = fn_ctx.in_loop;
    fn_ctx.in_loop = true;
    analyze_stmt(body, fn_ctx, table, target, ctx);
    fn_ctx.in_loop = saved;
    let cond_ty = check_expr(condition, table, target, ctx);
    require_scalar_condition(&cond_ty, expr_span_of(condition), "do-while", ctx);
}

#[allow(clippy::too_many_arguments)]
fn analyze_for(
    init: &Option<ForInit>,
    condition: Option<&Expr>,
    update: Option<&Expr>,
    body: &Stmt,
    fn_ctx: &mut FnContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    // A `for` statement's init clause introduces a new block scope that
    // extends across the entire loop (C17 §6.8.5p5).
    table.push_scope(ScopeKind::Block);
    match init {
        Some(ForInit::Declaration(d)) => analyze_declaration(d, table, target, ctx),
        Some(ForInit::Expr(e)) => {
            let _ = check_expr(e, table, target, ctx);
        }
        None => {}
    }
    if let Some(cond) = condition {
        let ty = check_expr(cond, table, target, ctx);
        require_scalar_condition(&ty, expr_span_of(cond), "for", ctx);
    }
    if let Some(upd) = update {
        let _ = check_expr(upd, table, target, ctx);
    }
    let saved = fn_ctx.in_loop;
    fn_ctx.in_loop = true;
    analyze_stmt(body, fn_ctx, table, target, ctx);
    fn_ctx.in_loop = saved;
    table.pop_scope();
}

// =========================================================================
// Switch / Case / Default
// =========================================================================

fn analyze_switch(
    controlling: &Expr,
    body: &Stmt,
    fn_ctx: &mut FnContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    let ty = check_expr(controlling, table, target, ctx);
    if !ty.ty.is_integer() {
        ctx.emit(
            Diagnostic::error("switch controlling expression must have integer type")
                .span(expr_span_of(controlling))
                .label(format!(
                    "type '{}' is not an integer",
                    ty.to_c_string(&ctx.type_ctx)
                )),
        );
    }
    let saved_in_switch = fn_ctx.in_switch;
    fn_ctx.in_switch = true;
    fn_ctx.switch_stack.push(SwitchInfo::default());
    analyze_stmt(body, fn_ctx, table, target, ctx);
    fn_ctx.switch_stack.pop();
    fn_ctx.in_switch = saved_in_switch;
}

fn analyze_case(
    value: &Expr,
    body: &Stmt,
    span: Span,
    fn_ctx: &mut FnContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    if !fn_ctx.in_switch {
        ctx.emit(Diagnostic::error("'case' label is not inside a switch statement").span(span));
    }
    // ICX evaluation is attempted regardless of in_switch so the user
    // sees both errors in a single pass.
    let evaluated = eval_icx_as_i64(value, table, target, ctx);
    if let Some(v) = evaluated {
        if let Some(info) = fn_ctx.switch_stack.last_mut() {
            if !info.cases_seen.insert(v) {
                ctx.emit(Diagnostic::error(format!("duplicate case value {v}")).span(span));
            }
        }
    }
    analyze_stmt(body, fn_ctx, table, target, ctx);
}

fn analyze_default(
    body: &Stmt,
    span: Span,
    fn_ctx: &mut FnContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    if !fn_ctx.in_switch {
        ctx.emit(Diagnostic::error("'default' label is not inside a switch statement").span(span));
    } else if let Some(info) = fn_ctx.switch_stack.last_mut() {
        if info.has_default {
            ctx.emit(Diagnostic::error("multiple default labels in one switch").span(span));
        } else {
            info.has_default = true;
        }
    }
    analyze_stmt(body, fn_ctx, table, target, ctx);
}

// =========================================================================
// Return / Break / Continue
// =========================================================================

fn analyze_return(
    value: Option<&Expr>,
    span: Span,
    fn_ctx: &mut FnContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    fn_ctx.return_seen = true;
    match value {
        None => {
            if !fn_ctx.return_type.ty.is_void() {
                ctx.emit(
                    Diagnostic::error("non-void function must return a value")
                        .span(span)
                        .note(format!(
                            "return type is '{}'",
                            fn_ctx.return_type.to_c_string(&ctx.type_ctx)
                        )),
                );
            }
        }
        Some(expr) => {
            let expr_ty = check_expr(expr, table, target, ctx);
            if fn_ctx.return_type.ty.is_void() {
                ctx.emit(
                    Diagnostic::error("void function cannot return a value")
                        .span(span)
                        .label(format!(
                            "returning value of type '{}'",
                            expr_ty.to_c_string(&ctx.type_ctx)
                        )),
                );
            }
            // Anything more specific than void-vs-nonvoid defers to the
            // richer assignment-compatibility checks that live in
            // crate::expr and will be invoked when the return path
            // lowers to an implicit conversion in a later prompt.
        }
    }
}

fn analyze_break(span: Span, fn_ctx: &FnContext, ctx: &mut SemaContext) {
    if !fn_ctx.in_loop && !fn_ctx.in_switch {
        ctx.emit(Diagnostic::error("'break' statement not in loop or switch").span(span));
    }
}

fn analyze_continue(span: Span, fn_ctx: &FnContext, ctx: &mut SemaContext) {
    // C17 §6.8.6.2: `continue` is a loop construct — a switch that is
    // *not* inside a loop does not satisfy the requirement.
    if !fn_ctx.in_loop {
        ctx.emit(Diagnostic::error("'continue' statement not in loop").span(span));
    }
}

// =========================================================================
// Label
// =========================================================================

fn analyze_label(
    name: &str,
    stmt: &Stmt,
    span: Span,
    fn_ctx: &mut FnContext,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    if let Some(prev) = fn_ctx.labels_defined.get(name) {
        ctx.emit(
            Diagnostic::error(format!("redefinition of label '{name}'"))
                .span(span)
                .label_at(*prev, "previously defined here"),
        );
    } else {
        fn_ctx.labels_defined.insert(name.to_string(), span);
    }
    analyze_stmt(stmt, fn_ctx, table, target, ctx);
}

// =========================================================================
// Condition helpers
// =========================================================================

fn require_scalar_condition(ty: &QualType, span: Span, kw: &str, ctx: &mut SemaContext) {
    if !ty.ty.is_scalar() {
        ctx.emit(
            Diagnostic::error(format!("'{kw}' condition must have scalar type",))
                .span(span)
                .label(format!(
                    "type '{}' is not scalar",
                    ty.to_c_string(&ctx.type_ctx)
                )),
        );
    }
}

fn warn_on_assignment_in_condition(expr: &Expr, ctx: &mut SemaContext) {
    if let Expr::Assignment {
        op: AssignOp::Assign,
        span,
        ..
    } = expr
    {
        ctx.emit(
            Diagnostic::warning(
                "assignment used as condition — did you mean '==' for equality comparison?",
            )
            .span(*span)
            .note("wrap the expression in an extra pair of parentheses to silence this warning"),
        );
    }
}

fn expr_span_of(expr: &Expr) -> Span {
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
// Function definition
// =========================================================================

/// Analyse a complete function definition: specifiers + declarator +
/// body.
///
/// Steps, in order:
///
/// 1. Resolve the declarator's return type and the function type.
/// 2. Register the function symbol in the enclosing (file) scope with
///    `is_defined = true`, merging any prior declaration via
///    [`SymbolTable::declare`].
/// 3. Push a fresh [`ScopeKind::Function`] scope.
/// 4. Declare each named parameter in that scope.
/// 5. Walk the body *in the same function scope* — the outermost
///    compound does not push a block scope (C17 §6.2.1p4).
/// 6. Resolve `goto` targets against the labels defined in the body.
/// 7. Emit a non-void-returns heuristic warning if the return type is
///    not `void` but no `return` statement was seen.
/// 8. Pop the function scope.
pub fn analyze_function_def(
    func: &FunctionDef,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    let Some(base_ty) = resolve_type_specifiers(&func.specifiers, table, target, ctx) else {
        return;
    };
    let Some((name_opt, ty)) =
        resolve_declarator(base_ty, &func.declarator, false, table, target, ctx)
    else {
        return;
    };
    let Some(name) = name_opt else {
        ctx.emit(Diagnostic::error("function definition must have an identifier").span(func.span));
        return;
    };

    // Extract the function payload for later use.  Anything else on the
    // left of the body is a misuse we diagnose and bail on.
    let (return_type, params, is_prototype) = match &ty.ty {
        Type::Function {
            return_type,
            params,
            is_prototype,
            ..
        } => ((**return_type).clone(), params.clone(), *is_prototype),
        _ => {
            ctx.emit(Diagnostic::error(format!("'{name}' is not a function")).span(func.span));
            return;
        }
    };

    // Storage class, inline, _Noreturn.
    let storage = convert_storage(func.specifiers.storage_class);
    let (is_inline, is_noreturn) = extract_function_specifiers(&func.specifiers);
    let linkage = function_linkage(storage);
    let ident_span = declarator_ident_span(&func.declarator).unwrap_or(func.span);

    // Register the function symbol in the ordinary namespace.  The
    // SymbolTable handles the composite-type merge and the
    // "redefinition of a previously-defined function" error.
    let sym = Symbol {
        id: 0,
        name: name.clone(),
        ty: ty.clone(),
        kind: SymbolKind::Function,
        storage,
        linkage,
        span: ident_span,
        is_defined: true,
        is_inline,
        is_noreturn,
        has_noreturn_attr: false,
    };
    let _ = table.declare(sym, ctx);

    // Reject K&R-style definitions that carry named parameters without
    // prototype information.  The parser accepts `int f() { ... }` as a
    // non-prototype with zero params (legal C17 legacy), so we only
    // reject *named* parameters in a non-prototype — a shape the parser
    // does not currently produce but we check defensively.
    if !is_prototype && !params.is_empty() {
        ctx.emit(
            Diagnostic::error("old-style (K&R) function definitions are not supported")
                .span(func.span),
        );
        return;
    }

    // Push the function scope and declare parameters.
    table.push_scope(ScopeKind::Function);
    declare_parameters(&params, &func.declarator.direct, func.span, table, ctx);

    // Walk the body directly — the outermost compound shares the
    // function scope per C17 §6.2.1p4.
    let mut fn_ctx = FnContext::new(return_type, is_noreturn);
    for item in &func.body.items {
        analyze_block_item(item, &mut fn_ctx, table, target, ctx);
    }

    // Resolve labels.  Every `goto` must reach a label that was defined
    // somewhere in the body; unused labels are permitted (C17 does not
    // require them to be referenced).
    for (lname, lspan) in &fn_ctx.labels_referenced {
        if !fn_ctx.labels_defined.contains_key(lname) {
            ctx.emit(Diagnostic::error(format!("use of undeclared label '{lname}'")).span(*lspan));
        }
    }

    // Non-void-returns heuristic: a function with a non-void return
    // type that never executes a `return` is almost certainly a bug.
    // `_Noreturn` functions are exempt — they legitimately never
    // return.  Functions that return `void` are exempt.
    if !fn_ctx.return_type.ty.is_void() && !fn_ctx.return_seen && !is_noreturn {
        ctx.emit(
            Diagnostic::warning(format!(
                "non-void function '{name}' does not return a value on any path",
            ))
            .span(func.span),
        );
    }

    table.pop_scope();
}

// =========================================================================
// Function-definition helpers
// =========================================================================

fn declare_parameters(
    params: &[ParamType],
    direct: &DirectDeclarator,
    fallback_span: Span,
    table: &mut SymbolTable,
    ctx: &mut SemaContext,
) {
    let param_decls = extract_param_decls(direct);
    for (idx, p) in params.iter().enumerate() {
        let Some(pname) = p.name.clone() else {
            if let Some(pd) = param_decls.and_then(|list| list.get(idx)) {
                ctx.emit(
                    Diagnostic::error("parameter name omitted in function definition")
                        .span(pd.span),
                );
            }
            continue;
        };
        // If no per-parameter declarator span is available (defensive
        // fallback — the parser normally supplies one per named param),
        // fall back to the function-definition span so the FileId is
        // correct for multi-file diagnostic rendering.
        let span = param_decls
            .and_then(|list| list.get(idx))
            .map_or(fallback_span, |pd| pd.span);
        if table.lookup_in_current_scope(&pname).is_some() {
            ctx.emit(Diagnostic::error(format!("redefinition of parameter '{pname}'")).span(span));
            continue;
        }
        let sym = Symbol {
            id: 0,
            name: pname,
            ty: p.ty.clone(),
            kind: SymbolKind::Parameter,
            storage: StorageClass::None,
            linkage: Linkage::None,
            span,
            is_defined: true,
            is_inline: false,
            is_noreturn: false,
            has_noreturn_attr: false,
        };
        let _ = table.declare(sym, ctx);
    }
}

/// Locate the outermost function-declarator payload within `direct` so
/// we can recover source spans for each parameter.  Returns `None` for
/// abstract or non-function shapes.
fn extract_param_decls(direct: &DirectDeclarator) -> Option<&[ParamDecl]> {
    match direct {
        DirectDeclarator::Function { params, .. } => Some(params.as_slice()),
        DirectDeclarator::Parenthesized(inner) => extract_param_decls(&inner.direct),
        DirectDeclarator::Array { .. } | DirectDeclarator::Identifier(_, _) => None,
    }
}

fn declarator_ident_span(decl: &forge_parser::ast::Declarator) -> Option<Span> {
    fn walk(d: &DirectDeclarator) -> Option<Span> {
        match d {
            DirectDeclarator::Identifier(_, span) => Some(*span),
            DirectDeclarator::Parenthesized(inner) => walk(&inner.direct),
            DirectDeclarator::Array { base, .. } | DirectDeclarator::Function { base, .. } => {
                walk(base)
            }
        }
    }
    walk(&decl.direct)
}

fn convert_storage(sc: Option<ParserStorageClass>) -> StorageClass {
    match sc {
        None => StorageClass::None,
        Some(ParserStorageClass::Auto) => StorageClass::Auto,
        Some(ParserStorageClass::Register) => StorageClass::Register,
        Some(ParserStorageClass::Static) => StorageClass::Static,
        Some(ParserStorageClass::Extern) => StorageClass::Extern,
        Some(ParserStorageClass::ThreadLocal) => StorageClass::ThreadLocal,
        // `typedef` is never valid on a function definition; callers
        // either prevented it or we fall through with the default.
        Some(ParserStorageClass::Typedef) => StorageClass::None,
    }
}

fn extract_function_specifiers(specs: &forge_parser::ast::DeclSpecifiers) -> (bool, bool) {
    let mut is_inline = false;
    let mut is_noreturn = false;
    for s in &specs.function_specifiers {
        match s {
            forge_parser::ast::FunctionSpecifier::Inline => is_inline = true,
            forge_parser::ast::FunctionSpecifier::Noreturn => is_noreturn = true,
        }
    }
    (is_inline, is_noreturn)
}

fn function_linkage(storage: StorageClass) -> Linkage {
    match storage {
        StorageClass::Static => Linkage::Internal,
        _ => Linkage::External,
    }
}
