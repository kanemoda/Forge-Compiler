//! Test root for `forge_diagnostics`.
//!
//! Tests are grouped by feature: the builder API, the ariadne-backed
//! renderer, and the [`crate::source_map`] types introduced for the
//! Phase 2 multi-file Span fix.

mod builder;
mod rendering;
mod source_map;
