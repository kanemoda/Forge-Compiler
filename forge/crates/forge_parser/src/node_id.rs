//! Stable per-parse AST node identifiers.
//!
//! Every AST variant that semantic analysis will annotate carries a
//! [`NodeId`] alongside its [`Span`](forge_lexer::Span).  Spans are
//! unsuitable as side-table keys because macro expansion can produce
//! multiple distinct AST nodes sharing a single source location — a
//! call to `MAX(a, b)` implemented as `((a) > (b) ? (a) : (b))`
//! instantiates several expression nodes with the same span, one per
//! reference to `a` or `b` in the macro body.
//!
//! `NodeId`s are assigned in parse order and are unique within a single
//! parse of a single translation unit.  They are **not** stable across
//! parses — in particular, they must not be persisted to disk for
//! incremental compilation; content-addressed keys should be used there
//! instead.
//!
//! # Test helpers
//!
//! Tests that hand-build AST nodes without going through the parser
//! should use [`NodeId::DUMMY`].  Phase 4 semantic analysis will never
//! see these nodes, so the collision on `u32::MAX` is harmless.

/// A parse-unique identifier for an AST node.
///
/// See the module-level docs for the uniqueness and stability
/// guarantees.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct NodeId(pub u32);

impl NodeId {
    /// Sentinel used by tests that build AST nodes by hand and never
    /// feed them into semantic analysis.  Real parser output never
    /// produces this value.
    pub const DUMMY: NodeId = NodeId(u32::MAX);
}
