//! Address-taken analysis for local variables.
//!
//! Phase 5 IR lowering needs to know which locals must live in memory
//! (`alloca` slots) versus which can stay in pure SSA registers.  A
//! local must be memory-resident whenever its address may have escaped
//! the function — i.e. whenever `&local` or array-to-pointer decay was
//! applied to it somewhere in the body.
//!
//! This pass runs once per function definition, after type-checking has
//! finished annotating side tables, and walks the entire body.  The
//! analysis is intentionally conservative: it marks every `&local` site
//! and every array-decay site, ignoring whether the resulting pointer
//! actually escapes (gets stored, returned, passed).  The optimizer
//! later recovers the rare cases where the address is computed but
//! never used.
//!
//! Only locals — [`SymbolKind::Variable`] with [`StorageClass::None`]
//! and [`Linkage::None`] — are tracked.  Parameters and globals are
//! left at their `false` default; Phase 5 treats both as unconditionally
//! memory-resident anyway, so a separate flag would be redundant.
//!
//! # Marking rules
//!
//! Two AST patterns mark a local as address-taken:
//!
//! 1. `Expr::UnaryOp { op: AddrOf, operand: Expr::Ident { … } }` —
//!    the classic `&x`.
//! 2. `Expr::Ident { … }` whose [`SemaContext::implicit_convs`] entry
//!    is [`ImplicitConversion::ArrayToPointer`] — the array decayed in
//!    a value context (passed as a pointer argument, used in pointer
//!    arithmetic, etc.).
//!
//! `sizeof` operands are *not* recursed into — their operand is
//! unevaluated per C17 §6.5.3.4, so an `&x` inside `sizeof` does not
//! produce a real pointer value at runtime.

use forge_parser::ast::{BlockItem, CompoundStmt, Designator, Expr, ForInit, Initializer, Stmt};
use forge_parser::ast_ops::UnaryOp;
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::scope::{Linkage, StorageClass, SymbolKind, SymbolTable};
use crate::types::ImplicitConversion;

/// Walk `body` and mark every local whose address may have escaped.
///
/// Called by [`crate::stmt::analyze_function_def`] after expression
/// type-checking has populated the side tables on `ctx`.  The function
/// scope must still be active when this runs so the walker's
/// `symbol_refs` lookups see the same symbol ids the type checker
/// recorded.
pub(crate) fn analyze_address_taken(
    body: &CompoundStmt,
    table: &mut SymbolTable,
    ctx: &SemaContext,
) {
    for item in &body.items {
        walk_block_item(item, table, ctx);
    }
}

fn walk_block_item(item: &BlockItem, table: &mut SymbolTable, ctx: &SemaContext) {
    match item {
        BlockItem::Declaration(d) => {
            for init_decl in &d.init_declarators {
                if let Some(init) = &init_decl.initializer {
                    walk_initializer(init, table, ctx);
                }
            }
        }
        BlockItem::Statement(s) => walk_stmt(s, table, ctx),
        BlockItem::StaticAssert(_) => {}
    }
}

fn walk_stmt(stmt: &Stmt, table: &mut SymbolTable, ctx: &SemaContext) {
    match stmt {
        Stmt::Compound(b) => {
            for item in &b.items {
                walk_block_item(item, table, ctx);
            }
        }
        Stmt::Expr { expr, .. } => {
            if let Some(e) = expr.as_deref() {
                walk_expr(e, table, ctx);
            }
        }
        Stmt::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            walk_expr(condition, table, ctx);
            walk_stmt(then_branch, table, ctx);
            if let Some(eb) = else_branch.as_deref() {
                walk_stmt(eb, table, ctx);
            }
        }
        Stmt::While {
            condition, body, ..
        } => {
            walk_expr(condition, table, ctx);
            walk_stmt(body, table, ctx);
        }
        Stmt::DoWhile {
            body, condition, ..
        } => {
            walk_stmt(body, table, ctx);
            walk_expr(condition, table, ctx);
        }
        Stmt::For {
            init,
            condition,
            update,
            body,
            ..
        } => {
            match init {
                Some(ForInit::Declaration(d)) => {
                    for init_decl in &d.init_declarators {
                        if let Some(i) = &init_decl.initializer {
                            walk_initializer(i, table, ctx);
                        }
                    }
                }
                Some(ForInit::Expr(e)) => walk_expr(e, table, ctx),
                None => {}
            }
            if let Some(c) = condition.as_deref() {
                walk_expr(c, table, ctx);
            }
            if let Some(u) = update.as_deref() {
                walk_expr(u, table, ctx);
            }
            walk_stmt(body, table, ctx);
        }
        Stmt::Switch { expr, body, .. } => {
            walk_expr(expr, table, ctx);
            walk_stmt(body, table, ctx);
        }
        Stmt::Case { value, body, .. } => {
            walk_expr(value, table, ctx);
            walk_stmt(body, table, ctx);
        }
        Stmt::Default { body, .. } => walk_stmt(body, table, ctx),
        Stmt::Return { value, .. } => {
            if let Some(v) = value.as_deref() {
                walk_expr(v, table, ctx);
            }
        }
        Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Goto { .. } => {}
        Stmt::Label { stmt, .. } => walk_stmt(stmt, table, ctx),
    }
}

fn walk_initializer(init: &Initializer, table: &mut SymbolTable, ctx: &SemaContext) {
    match init {
        Initializer::Expr(e) => walk_expr(e, table, ctx),
        Initializer::List { items, .. } => {
            for di in items {
                for d in &di.designators {
                    if let Designator::Index(e) = d {
                        walk_expr(e, table, ctx);
                    }
                }
                walk_initializer(&di.initializer, table, ctx);
            }
        }
    }
}

fn walk_expr(e: &Expr, table: &mut SymbolTable, ctx: &SemaContext) {
    // Rule 2: an identifier that decayed via array-to-pointer marks the
    // underlying local.  We check this on every Expr node, but only an
    // `Expr::Ident` ever carries an `ArrayToPointer` conversion.
    if let Expr::Ident { node_id, .. } = e {
        if matches!(
            ctx.implicit_convs.get(&node_id.0),
            Some(ImplicitConversion::ArrayToPointer)
        ) {
            mark_if_local(*node_id, table, ctx);
        }
    }

    match e {
        // Rule 1: `&E` where E is an Ident resolving to a local.
        Expr::UnaryOp {
            op: UnaryOp::AddrOf,
            operand,
            ..
        } => {
            if let Expr::Ident { node_id, .. } = operand.as_ref() {
                mark_if_local(*node_id, table, ctx);
            }
            // Recurse into the operand so nested `&` or array-decay
            // sites (e.g. `&arr[i]`) are still visited.
            walk_expr(operand, table, ctx);
        }

        // Leaves and identifier nodes — nothing further to recurse into.
        Expr::IntLiteral { .. }
        | Expr::FloatLiteral { .. }
        | Expr::CharLiteral { .. }
        | Expr::StringLiteral { .. }
        | Expr::Ident { .. } => {}

        Expr::BinaryOp { left, right, .. } => {
            walk_expr(left, table, ctx);
            walk_expr(right, table, ctx);
        }
        Expr::UnaryOp { operand, .. } => walk_expr(operand, table, ctx),
        Expr::PostfixOp { operand, .. } => walk_expr(operand, table, ctx),
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            walk_expr(condition, table, ctx);
            walk_expr(then_expr, table, ctx);
            walk_expr(else_expr, table, ctx);
        }
        Expr::Assignment { target, value, .. } => {
            walk_expr(target, table, ctx);
            walk_expr(value, table, ctx);
        }
        Expr::FunctionCall { callee, args, .. } => {
            walk_expr(callee, table, ctx);
            for a in args {
                walk_expr(a, table, ctx);
            }
        }
        Expr::MemberAccess { object, .. } => walk_expr(object, table, ctx),
        Expr::ArraySubscript { array, index, .. } => {
            walk_expr(array, table, ctx);
            walk_expr(index, table, ctx);
        }
        Expr::Cast { expr, .. } => walk_expr(expr, table, ctx),

        // Operands of `sizeof` / `_Alignof` are unevaluated.  The
        // pointer they would produce never materialises at runtime, so
        // we deliberately skip recursion here.
        Expr::SizeofExpr { .. } | Expr::SizeofType { .. } | Expr::AlignofType { .. } => {}

        Expr::CompoundLiteral { initializer, .. } => {
            walk_initializer(initializer, table, ctx);
        }
        Expr::GenericSelection {
            controlling,
            associations,
            ..
        } => {
            walk_expr(controlling, table, ctx);
            for a in associations {
                walk_expr(&a.expr, table, ctx);
            }
        }
        Expr::Comma { exprs, .. } => {
            for e in exprs {
                walk_expr(e, table, ctx);
            }
        }
        Expr::BuiltinOffsetof { .. } | Expr::BuiltinTypesCompatibleP { .. } => {}
    }
}

/// If `ident_node_id` resolves to a local variable in scope, set its
/// `address_taken` flag.  Globals, parameters, functions, typedefs, and
/// enum constants are silently ignored — only true locals are tracked.
fn mark_if_local(ident_node_id: NodeId, table: &mut SymbolTable, ctx: &SemaContext) {
    if ident_node_id == NodeId::DUMMY {
        return;
    }
    let Some(&sym_id) = ctx.symbol_refs.get(&ident_node_id.0) else {
        return;
    };
    let sym = table.symbol(sym_id);
    let is_local = matches!(sym.kind, SymbolKind::Variable)
        && matches!(sym.storage, StorageClass::None)
        && matches!(sym.linkage, Linkage::None);
    if is_local {
        table.mark_address_taken(sym_id);
    }
}
