#![allow(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::doc_markdown
)]

//! Semantic analysis for the Forge C17 compiler.
//!
//! Phase 4 of the pipeline.  This crate consumes the syntactic AST from
//! [`forge_parser`] and annotates it with resolved types, scope
//! information, and implicit conversions.  The present module exposes
//! only the core **type system** — the representation of C17 types, the
//! implicit-conversion algebra (integer promotions, usual arithmetic
//! conversions), compatibility, composite types, and target metadata.
//!
//! Symbol resolution, scope walking, and full expression type-checking
//! live in follow-up phases and build on top of these types.
//!
//! # Entry points
//!
//! * [`Type`] / [`QualType`] — the core type representation.
//! * [`TargetInfo`] — ABI-specific sizes and alignments.
//! * [`integer_promotion`], [`usual_arithmetic_conversions`] — the two
//!   functions every arithmetic expression goes through.
//! * [`are_compatible`] / [`composite_type`] — C17 §6.2.7.

pub mod const_eval;
pub mod context;
pub mod declare;
pub mod expr;
pub mod layout;
pub mod resolve;
pub mod scope;
pub mod stmt;
pub mod tu;
pub mod types;

#[cfg(test)]
mod tests;

pub use const_eval::{eval_icx, eval_icx_as_i64, eval_icx_as_u64, ConstValue};
pub use context::SemaContext;
pub use declare::{analyze_declaration, analyze_static_assert, check_initializer};
pub use expr::{check_expr, check_expr_in_context, ValueContext};
pub use layout::{complete_enum, complete_struct, complete_union};
pub use resolve::{declarator_name, resolve_declarator, resolve_type_specifiers};
pub use scope::{
    Linkage, Scope, ScopeKind, StorageClass, Symbol, SymbolId, SymbolKind, SymbolTable, TagEntry,
    TagId,
};
pub use stmt::{analyze_function_def, analyze_stmt, FnContext, SwitchInfo};
pub use tu::analyze_translation_unit;
pub use types::{
    are_compatible, are_compatible_unqualified, composite_type, integer_promotion,
    usual_arithmetic_conversions, AnonMemberMap, ArraySize, BitFieldLayout, EnumLayout, EnumTypeId,
    ImplicitConversion, MemberLayout, ParamType, QualType, Signedness, SizeofKind, StructLayout,
    StructTypeId, TargetInfo, Type, TypeContext, UnionLayout, UnionTypeId,
};
