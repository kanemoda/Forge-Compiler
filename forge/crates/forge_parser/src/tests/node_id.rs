//! Sanity check that every AST node the parser emits gets a unique,
//! densely-packed [`NodeId`].
//!
//! Semantic analysis keys its side tables on `NodeId`, so collisions or
//! gaps in the numbering would cause annotations to be lost or mis-
//! attributed.

use std::collections::HashSet;

use super::helpers::*;
use crate::ast::*;
use crate::node_id::NodeId;

#[test]
fn node_ids_are_unique_and_dense() {
    let tu = parse_tu("int main(void) { int x = 1 + 2; return x; }");

    let mut ids: Vec<NodeId> = Vec::new();
    collect_tu(&tu, &mut ids);

    assert!(!ids.is_empty(), "expected the parser to emit node ids");

    let unique: HashSet<_> = ids.iter().copied().collect();
    assert_eq!(
        unique.len(),
        ids.len(),
        "node ids must be unique — duplicates = {}",
        ids.len() - unique.len()
    );

    assert!(
        ids.iter().all(|id| *id != NodeId::DUMMY),
        "real parser output must never produce NodeId::DUMMY"
    );

    let max = ids.iter().map(|NodeId(v)| *v).max().expect("non-empty");
    assert_eq!(
        max as usize + 1,
        ids.len(),
        "node ids must be dense (0..N contiguous): max = {max}, count = {}",
        ids.len()
    );
}

// =========================================================================
// AST walker — collects every node_id field the parser fills in.
// =========================================================================

fn collect_tu(tu: &TranslationUnit, out: &mut Vec<NodeId>) {
    for d in &tu.declarations {
        collect_external(d, out);
    }
}

fn collect_external(d: &ExternalDeclaration, out: &mut Vec<NodeId>) {
    match d {
        ExternalDeclaration::FunctionDef(f) => collect_function(f, out),
        ExternalDeclaration::Declaration(decl) => collect_declaration(decl, out),
        ExternalDeclaration::StaticAssert(sa) => collect_expr(&sa.condition, out),
    }
}

fn collect_function(f: &FunctionDef, out: &mut Vec<NodeId>) {
    out.push(f.node_id);
    for item in &f.body.items {
        collect_block_item(item, out);
    }
}

fn collect_declaration(d: &Declaration, out: &mut Vec<NodeId>) {
    out.push(d.node_id);
    for init in &d.init_declarators {
        out.push(init.node_id);
        if let Some(i) = &init.initializer {
            collect_initializer(i, out);
        }
    }
}

fn collect_block_item(item: &BlockItem, out: &mut Vec<NodeId>) {
    match item {
        BlockItem::Statement(s) => collect_stmt(s, out),
        BlockItem::Declaration(d) => collect_declaration(d, out),
        BlockItem::StaticAssert(sa) => collect_expr(&sa.condition, out),
    }
}

fn collect_stmt(s: &Stmt, out: &mut Vec<NodeId>) {
    match s {
        Stmt::Compound(cs) => {
            for item in &cs.items {
                collect_block_item(item, out);
            }
        }
        Stmt::Expr { node_id, expr, .. } => {
            out.push(*node_id);
            if let Some(e) = expr {
                collect_expr(e, out);
            }
        }
        Stmt::If {
            node_id,
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            out.push(*node_id);
            collect_expr(condition, out);
            collect_stmt(then_branch, out);
            if let Some(e) = else_branch {
                collect_stmt(e, out);
            }
        }
        Stmt::While {
            node_id,
            condition,
            body,
            ..
        } => {
            out.push(*node_id);
            collect_expr(condition, out);
            collect_stmt(body, out);
        }
        Stmt::DoWhile {
            node_id,
            body,
            condition,
            ..
        } => {
            out.push(*node_id);
            collect_stmt(body, out);
            collect_expr(condition, out);
        }
        Stmt::For {
            node_id,
            init,
            condition,
            update,
            body,
            ..
        } => {
            out.push(*node_id);
            match init {
                Some(ForInit::Declaration(d)) => collect_declaration(d, out),
                Some(ForInit::Expr(e)) => collect_expr(e, out),
                None => {}
            }
            if let Some(c) = condition {
                collect_expr(c, out);
            }
            if let Some(u) = update {
                collect_expr(u, out);
            }
            collect_stmt(body, out);
        }
        Stmt::Switch {
            node_id,
            expr,
            body,
            ..
        } => {
            out.push(*node_id);
            collect_expr(expr, out);
            collect_stmt(body, out);
        }
        Stmt::Case {
            node_id,
            value,
            body,
            ..
        } => {
            out.push(*node_id);
            collect_expr(value, out);
            collect_stmt(body, out);
        }
        Stmt::Default { node_id, body, .. } => {
            out.push(*node_id);
            collect_stmt(body, out);
        }
        Stmt::Return { node_id, value, .. } => {
            out.push(*node_id);
            if let Some(v) = value {
                collect_expr(v, out);
            }
        }
        Stmt::Break { node_id, .. }
        | Stmt::Continue { node_id, .. }
        | Stmt::Goto { node_id, .. } => {
            out.push(*node_id);
        }
        Stmt::Label { node_id, stmt, .. } => {
            out.push(*node_id);
            collect_stmt(stmt, out);
        }
    }
}

fn collect_expr(e: &Expr, out: &mut Vec<NodeId>) {
    match e {
        Expr::IntLiteral { node_id, .. }
        | Expr::FloatLiteral { node_id, .. }
        | Expr::CharLiteral { node_id, .. }
        | Expr::StringLiteral { node_id, .. }
        | Expr::Ident { node_id, .. } => {
            out.push(*node_id);
        }
        Expr::BinaryOp {
            node_id,
            left,
            right,
            ..
        } => {
            out.push(*node_id);
            collect_expr(left, out);
            collect_expr(right, out);
        }
        Expr::UnaryOp {
            node_id, operand, ..
        }
        | Expr::PostfixOp {
            node_id, operand, ..
        } => {
            out.push(*node_id);
            collect_expr(operand, out);
        }
        Expr::Conditional {
            node_id,
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            out.push(*node_id);
            collect_expr(condition, out);
            collect_expr(then_expr, out);
            collect_expr(else_expr, out);
        }
        Expr::Assignment {
            node_id,
            target,
            value,
            ..
        } => {
            out.push(*node_id);
            collect_expr(target, out);
            collect_expr(value, out);
        }
        Expr::FunctionCall {
            node_id,
            callee,
            args,
            ..
        } => {
            out.push(*node_id);
            collect_expr(callee, out);
            for a in args {
                collect_expr(a, out);
            }
        }
        Expr::MemberAccess {
            node_id, object, ..
        } => {
            out.push(*node_id);
            collect_expr(object, out);
        }
        Expr::ArraySubscript {
            node_id,
            array,
            index,
            ..
        } => {
            out.push(*node_id);
            collect_expr(array, out);
            collect_expr(index, out);
        }
        Expr::Cast {
            node_id,
            type_name,
            expr,
            ..
        } => {
            out.push(*node_id);
            out.push(type_name.node_id);
            collect_expr(expr, out);
        }
        Expr::SizeofExpr { node_id, expr, .. } => {
            out.push(*node_id);
            collect_expr(expr, out);
        }
        Expr::SizeofType {
            node_id, type_name, ..
        }
        | Expr::AlignofType {
            node_id, type_name, ..
        } => {
            out.push(*node_id);
            out.push(type_name.node_id);
        }
        Expr::CompoundLiteral {
            node_id,
            type_name,
            initializer,
            ..
        } => {
            out.push(*node_id);
            out.push(type_name.node_id);
            collect_initializer(initializer, out);
        }
        Expr::GenericSelection {
            node_id,
            controlling,
            associations,
            ..
        } => {
            out.push(*node_id);
            collect_expr(controlling, out);
            for a in associations {
                if let Some(tn) = &a.type_name {
                    out.push(tn.node_id);
                }
                collect_expr(&a.expr, out);
            }
        }
        Expr::Comma { node_id, exprs, .. } => {
            out.push(*node_id);
            for e in exprs {
                collect_expr(e, out);
            }
        }
        Expr::BuiltinOffsetof {
            node_id,
            ty,
            designator,
            ..
        } => {
            out.push(*node_id);
            out.push(ty.node_id);
            for step in designator {
                if let OffsetofMember::Subscript(idx) = step {
                    collect_expr(idx, out);
                }
            }
        }
        Expr::BuiltinTypesCompatibleP {
            node_id, t1, t2, ..
        } => {
            out.push(*node_id);
            out.push(t1.node_id);
            out.push(t2.node_id);
        }
    }
}

fn collect_initializer(i: &Initializer, out: &mut Vec<NodeId>) {
    match i {
        Initializer::Expr(e) => collect_expr(e, out),
        Initializer::List { node_id, items, .. } => {
            out.push(*node_id);
            for item in items {
                for d in &item.designators {
                    if let Designator::Index(e) = d {
                        collect_expr(e, out);
                    }
                }
                collect_initializer(&item.initializer, out);
            }
        }
    }
}
