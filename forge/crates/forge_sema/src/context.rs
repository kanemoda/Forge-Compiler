//! Global per-translation-unit state for semantic analysis.
//!
//! A single [`SemaContext`] is threaded through every sema function
//! and owns:
//!
//! * the [`TypeContext`] (struct / union / enum layout side table),
//! * the running diagnostic list, and
//! * the per-AST-node side tables: resolved expression types, inserted
//!   implicit conversions, symbol references, lvalue-ness, and the
//!   lowering directives for each `sizeof` operand.
//!
//! Keeping all of this behind one handle means each sema function takes
//! one `&mut SemaContext` rather than fanning out to parallel
//! arguments.  The [`SymbolTable`] and [`TargetInfo`] are threaded in
//! alongside — the symbol table because it is mutated during scope
//! walking and Rust's borrow checker needs a clean split from the side
//! tables on `SemaContext`, and the target because it is stable for the
//! whole translation unit.
//!
//! ## Side tables
//!
//! The parser tags every expression, identifier, and `sizeof` operand
//! with a [`NodeId`].  Sema publishes its per-node results on the
//! matching side table so later phases (IR lowering, diagnostics) can
//! look them up by id without re-deriving the analysis.
//!
//! * [`SemaContext::expr_types`] is a dense `Vec<Option<QualType>>`
//!   indexed by `NodeId.0` because every expression gets a type.
//! * [`SemaContext::implicit_convs`], [`SemaContext::symbol_refs`],
//!   [`SemaContext::lvalues`], and [`SemaContext::sizeof_kinds`] are
//!   sparse `FxHashMap`/`FxHashSet` keyed by `NodeId.0` — only a
//!   minority of nodes record entries in each.

use forge_diagnostics::{Diagnostic, Severity};
use forge_parser::NodeId;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::scope::SymbolId;
use crate::types::{ImplicitConversion, QualType, SizeofKind, TargetInfo, TypeContext};

/// Mutable per-translation-unit sema state.
///
/// Owns the [`TypeContext`] that every type query consults, the list of
/// diagnostics that sema accumulates, and the side tables that publish
/// each AST node's analysis result.  The [`SymbolTable`] and
/// [`TargetInfo`] travel separately so sema functions can borrow them
/// alongside `&mut SemaContext` without borrow-checker conflicts.
///
/// [`SymbolTable`]: crate::scope::SymbolTable
#[derive(Debug)]
pub struct SemaContext {
    /// Struct / union / enum layout side table.
    pub type_ctx: TypeContext,
    /// Accumulated diagnostics, in source order.
    pub diagnostics: Vec<Diagnostic>,
    /// Dense per-expression resolved type.  Indexed by `NodeId.0`.
    /// Entries are `None` before sema visits the node and `Some` after.
    pub expr_types: Vec<Option<QualType>>,
    /// Sparse: implicit conversion inserted at a given expression node.
    pub implicit_convs: FxHashMap<u32, ImplicitConversion>,
    /// Sparse: identifier expression → the [`SymbolId`] it resolved to.
    pub symbol_refs: FxHashMap<u32, SymbolId>,
    /// Sparse: expression nodes that are lvalues.
    pub lvalues: FxHashSet<u32>,
    /// Sparse: lowering directive for a `sizeof` operand.
    pub sizeof_kinds: FxHashMap<u32, SizeofKind>,
}

impl SemaContext {
    /// Build an empty context.
    pub fn new() -> Self {
        Self {
            type_ctx: TypeContext::default(),
            diagnostics: Vec::new(),
            expr_types: Vec::new(),
            implicit_convs: FxHashMap::default(),
            symbol_refs: FxHashMap::default(),
            lvalues: FxHashSet::default(),
            sizeof_kinds: FxHashMap::default(),
        }
    }

    /// Shorthand for `self.diagnostics.push(diag)`.
    pub fn emit(&mut self, diag: Diagnostic) {
        self.diagnostics.push(diag);
    }

    /// Emit an error-severity diagnostic.  Equivalent to
    /// `self.emit(Diagnostic::error(msg))` but shorter at call sites.
    pub fn error(&mut self, msg: impl Into<String>) {
        self.emit(Diagnostic::error(msg));
    }

    /// Emit a warning-severity diagnostic.
    pub fn warn(&mut self, msg: impl Into<String>) {
        self.emit(Diagnostic::warning(msg));
    }

    /// `true` if at least one error-severity diagnostic has been emitted.
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    /// Record the resolved type of an expression node.
    ///
    /// Grows `expr_types` as needed; callers do not need to pre-size
    /// the vector.  The [`NodeId::DUMMY`] sentinel (used by hand-built
    /// AST nodes in tests) is silently ignored.
    pub fn set_type(&mut self, node: NodeId, ty: QualType) {
        if node == NodeId::DUMMY {
            return;
        }
        let idx = node.0 as usize;
        if self.expr_types.len() <= idx {
            self.expr_types.resize(idx + 1, None);
        }
        self.expr_types[idx] = Some(ty);
    }

    /// Look up the resolved type of an expression node.  Returns `None`
    /// if sema has not visited the node yet (or if the id is
    /// [`NodeId::DUMMY`]).
    pub fn get_type(&self, node: NodeId) -> Option<&QualType> {
        if node == NodeId::DUMMY {
            return None;
        }
        self.expr_types.get(node.0 as usize)?.as_ref()
    }

    /// Record an implicit conversion inserted at this expression.
    pub fn set_implicit_conv(&mut self, node: NodeId, conv: ImplicitConversion) {
        if node == NodeId::DUMMY {
            return;
        }
        self.implicit_convs.insert(node.0, conv);
    }

    /// Record that an identifier expression resolved to the given symbol.
    pub fn set_symbol_ref(&mut self, node: NodeId, sym: SymbolId) {
        if node == NodeId::DUMMY {
            return;
        }
        self.symbol_refs.insert(node.0, sym);
    }

    /// Mark an expression node as an lvalue.
    pub fn mark_lvalue(&mut self, node: NodeId) {
        if node == NodeId::DUMMY {
            return;
        }
        self.lvalues.insert(node.0);
    }

    /// `true` if the node has been marked as an lvalue.
    pub fn is_lvalue(&self, node: NodeId) -> bool {
        if node == NodeId::DUMMY {
            return false;
        }
        self.lvalues.contains(&node.0)
    }

    /// Record how a `sizeof` operand should be lowered.
    pub fn set_sizeof_kind(&mut self, node: NodeId, kind: SizeofKind) {
        if node == NodeId::DUMMY {
            return;
        }
        self.sizeof_kinds.insert(node.0, kind);
    }
}

impl Default for SemaContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience alias — every sema function takes a target by reference.
pub type Target<'a> = &'a TargetInfo;
