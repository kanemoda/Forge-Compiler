//! Macro-expansion frames and the table the preprocessor populates.
//!
//! The preprocessor records one [`ExpansionFrame`] per macro invocation
//! it processes.  Every token produced by that invocation's replacement
//! list gets its [`Span::expanded_from`](crate::Span::expanded_from)
//! stamped with the frame's [`ExpansionId`].  When the renderer later
//! formats a diagnostic whose primary span points inside an expansion,
//! it walks the `parent` chain of that id to emit an "in expansion of
//! macro 'M'" note for every frame — giving the user the full call
//! chain back to the original source.
//!
//! ## Placement note
//!
//! The [`Section 1 dependency decision`](../../../docs/phases) in the
//! Phase 2F.3 prompt suggested keeping [`ExpansionTable`] inside
//! `forge_preprocess`.  We instead keep both [`ExpansionFrame`] and
//! [`ExpansionTable`] here for the same reason [`crate::SourceMap`]
//! lives here: the renderer needs to read them, and every field is
//! already a type defined in this crate.  Putting the table in the
//! preprocessor would either force a trait wrapper or a back-reference
//! from `forge_diagnostics` to `forge_preprocess` (which would be
//! circular).  The preprocessor re-exports these types so day-to-day
//! usage reads `forge_preprocess::ExpansionTable` as the spec intended.

use crate::{ExpansionId, Span};

/// A single macro-expansion record.
///
/// Frames are append-only: once pushed, their index in the table is the
/// [`ExpansionId`] used by every stamped span.  The order of frames in
/// the table reflects the order in which the preprocessor processed
/// macro invocations; it is **not** a call-tree ordering — follow the
/// `parent` field to reconstruct enclosing invocations.
#[derive(Debug, Clone)]
pub struct ExpansionFrame {
    /// The id this frame was assigned when pushed onto its table.
    pub id: ExpansionId,
    /// The span where the macro name appeared, pre-expansion.  For
    /// function-like macros this points at the macro-name identifier;
    /// for object-like macros it is the same.  For builtin expansions
    /// (`__LINE__`, `__FILE__`, …) it is the use-site of the builtin.
    pub invocation_span: Span,
    /// The macro's name as typed at the invocation site (e.g. `"PI"` or
    /// `"__LINE__"`).  Used verbatim in rendered "in expansion of macro
    /// 'M'" notes.
    pub macro_name: String,
    /// The span of the `#define` line that introduced this macro — or a
    /// default/synthetic span for compiler-provided builtins.
    pub definition_span: Span,
    /// Id of the enclosing expansion, or [`ExpansionId::NONE`] for a
    /// top-level invocation that appeared in real source text.
    pub parent: ExpansionId,
}

/// Append-only table of every expansion frame produced during a single
/// preprocessing run.
///
/// [`ExpansionId`]s are allocated sequentially starting at `0`; the
/// [`ExpansionId::NONE`] sentinel is `u32::MAX` and is never returned by
/// [`ExpansionTable::push`].
#[derive(Debug, Clone, Default)]
pub struct ExpansionTable {
    frames: Vec<ExpansionFrame>,
}

impl ExpansionTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self { frames: Vec::new() }
    }

    /// Append a new frame and return the [`ExpansionId`] it was given.
    ///
    /// The caller provides the frame's payload in `frame`, but the
    /// frame's `id` field is always overwritten with the newly
    /// allocated id before the frame is stored — so callers can pass
    /// `ExpansionId::NONE` (or any placeholder) and rely on the
    /// returned id being the authoritative one.
    pub fn push(&mut self, mut frame: ExpansionFrame) -> ExpansionId {
        let id = ExpansionId(self.frames.len() as u32);
        frame.id = id;
        self.frames.push(frame);
        id
    }

    /// Look up a frame by id.  Returns [`None`] for
    /// [`ExpansionId::NONE`] or any id past the end of the table.
    pub fn get(&self, id: ExpansionId) -> Option<&ExpansionFrame> {
        if !id.is_some() {
            return None;
        }
        self.frames.get(id.0 as usize)
    }

    /// Number of frames in the table.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether no frames have been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Iterate over every frame in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &ExpansionFrame> {
        self.frames.iter()
    }

    /// Walk the parent chain starting at `id`, returning the frames
    /// encountered — innermost (`id`) first, outermost last.
    ///
    /// If `id` is [`ExpansionId::NONE`] or refers to a missing frame,
    /// the returned slice is empty.  A cycle in the parent chain (which
    /// should never happen in a correctly-written preprocessor) is
    /// defensively truncated at the table's current length so this
    /// method never loops indefinitely.
    pub fn backtrace(&self, id: ExpansionId) -> Vec<&ExpansionFrame> {
        let mut chain = Vec::new();
        let mut cursor = id;
        let mut guard = self.frames.len() + 1;
        while let Some(frame) = self.get(cursor) {
            chain.push(frame);
            if guard == 0 {
                break;
            }
            guard -= 1;
            cursor = frame.parent;
        }
        chain
    }
}
