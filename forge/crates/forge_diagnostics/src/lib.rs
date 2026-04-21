// Pedantic lints we've audited and accept as style preferences for this crate.
#![allow(
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::unreadable_literal,
    clippy::doc_markdown,
    clippy::wildcard_imports,
    clippy::needless_pass_by_value,
    clippy::manual_let_else
)]

//! Diagnostic reporting for the Forge compiler.
//!
//! This crate provides the [`Diagnostic`] type and the [`render_diagnostics`]
//! / [`render_diagnostics_to_string`] functions, which use the `ariadne` crate
//! to produce human-readable, source-annotated error messages.
//!
//! # Span type
//!
//! Every diagnostic and label carries a [`Span`] that includes the
//! [`FileId`] of the source it references, so a single diagnostic can
//! point at labels across `#include`-expanded files. The renderer
//! consults the supplied [`SourceMap`] to turn those ids back into file
//! names and source text.
//!
//! # Builder pattern
//!
//! Diagnostics are constructed via a fluent builder API.  Calling
//! `Diagnostic::error` / `Diagnostic::warning` creates a diagnostic with only
//! a message.  The primary source span is attached with `.span()`, after which
//! `.label()`, `.label_at()`, and `.note()` can be chained.

use std::fmt;
use std::io::IsTerminal;
use std::ops::Range;

use ariadne::{Cache, Color, Config, Label as AriadneLabel, Report, ReportKind, Source};

pub mod expansion;
pub mod source_map;

pub use expansion::{ExpansionFrame, ExpansionTable};
pub use source_map::{FileId, SourceFile, SourceMap};

// ---------------------------------------------------------------------------
// ExpansionId
// ---------------------------------------------------------------------------

/// Index into a preprocessor-owned expansion table.
///
/// A [`Span`] whose `expanded_from` is not [`ExpansionId::NONE`] was
/// produced by a macro expansion; the id points at the innermost
/// expansion frame that stamped it.  Walking the frame's `parent` chain
/// yields the enclosing invocations, in innermost-to-outermost order.
///
/// The concrete frame table (`ExpansionTable`) lives in
/// `forge_preprocess` — this crate only carries the id so that `Span`
/// can reference expansions without forcing `forge_lexer` (which does
/// not know about macros) to depend on the preprocessor.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct ExpansionId(pub u32);

impl ExpansionId {
    /// Sentinel meaning "this span was not produced by macro expansion".
    pub const NONE: ExpansionId = ExpansionId(u32::MAX);

    /// Whether this id names a real frame (i.e. is not [`Self::NONE`]).
    pub const fn is_some(self) -> bool {
        self.0 != Self::NONE.0
    }
}

// ---------------------------------------------------------------------------
// Span
// ---------------------------------------------------------------------------

/// A byte range inside a specific source file.
///
/// `Span` is the shared span type between the lexer, preprocessor, parser,
/// and diagnostics layers. It lives in `forge_diagnostics` (and is
/// re-exported from `forge_lexer`) so every crate above the lexer sees
/// the same representation.
///
/// In addition to the (file, start, end) triple, a span carries an
/// [`ExpansionId`] naming the macro expansion the token came from — or
/// [`ExpansionId::NONE`] for tokens lexed straight from the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    /// The file this span points into.
    pub file: FileId,
    /// Inclusive start byte offset.
    pub start: u32,
    /// Exclusive end byte offset.
    pub end: u32,
    /// The expansion that produced this token, or [`ExpansionId::NONE`]
    /// for tokens that came directly from the source.
    pub expanded_from: ExpansionId,
}

impl Span {
    /// Build a span pointing into the given file.  `expanded_from` is
    /// initialised to [`ExpansionId::NONE`]; use
    /// [`Span::with_expansion`] to stamp an expansion id.
    pub const fn new(file: FileId, start: u32, end: u32) -> Self {
        Self {
            file,
            start,
            end,
            expanded_from: ExpansionId::NONE,
        }
    }

    /// Convenience for tests and single-file fixtures: build a span in
    /// the primary (first) file of a `SourceMap`.
    pub const fn primary(start: u32, end: u32) -> Self {
        Self::new(FileId::PRIMARY, start, end)
    }

    /// Return `self` with the given expansion id attached.  Chainable
    /// with the other constructors so call sites can write
    /// `Span::new(f, a, b).with_expansion(id)`.
    pub const fn with_expansion(mut self, id: ExpansionId) -> Self {
        self.expanded_from = id;
        self
    }

    /// Byte length of the span.
    pub const fn len(&self) -> u32 {
        self.end - self.start
    }

    /// Whether the span covers zero bytes.
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Convert to a `Range<usize>` for APIs (e.g., `ariadne`) that still
    /// want a raw byte range.
    pub fn range(&self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}..{}", self.file.0, self.start, self.end)
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The severity level of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A hard error; compilation cannot succeed.
    Error,
    /// A warning; compilation can continue but the user should be aware.
    Warning,
    /// An informational note, typically attached to another diagnostic.
    Note,
}

/// A source-level label pointing to a specific span with an explanatory message.
#[derive(Debug, Clone)]
pub struct Label {
    /// The source span this label points to.
    pub span: Span,
    /// The message displayed alongside the underlined span.
    pub message: String,
}

/// A single compiler diagnostic (error, warning, or note).
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// The primary human-readable message describing what went wrong.
    pub message: String,
    /// The primary source span.
    ///
    /// Defaults to an all-zero span pointing at [`FileId::INVALID`] when
    /// the diagnostic has not yet had a span attached via
    /// [`.span()`](Diagnostic::span).
    pub span: Span,
    /// How severe this diagnostic is.
    pub severity: Severity,
    /// Additional source labels pointing at relevant locations.
    pub labels: Vec<Label>,
    /// Free-form notes appended below the diagnostic (e.g., suggestions).
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Builder API
// ---------------------------------------------------------------------------

impl Diagnostic {
    /// Start building an **error** diagnostic with the given message.
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message)
    }

    /// Start building a **warning** diagnostic with the given message.
    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message)
    }

    /// Start building a **note** diagnostic with the given message.
    pub fn note_diag(message: impl Into<String>) -> Self {
        Self::new(Severity::Note, message)
    }

    fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: Span::new(FileId::INVALID, 0, 0),
            severity,
            labels: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Set the primary source span and return `self` for chaining.
    pub fn span(mut self, span: Span) -> Self {
        self.span = span;
        self
    }

    /// Add a label pointing at the **primary span** with the given message,
    /// and return `self` for chaining.
    pub fn label(mut self, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span: self.span,
            message: message.into(),
        });
        self
    }

    /// Add a label at an **explicit span** and return `self` for chaining.
    pub fn label_at(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            message: message.into(),
        });
        self
    }

    /// Append a free-form note and return `self` for chaining.
    pub fn note(mut self, message: impl Into<String>) -> Self {
        self.notes.push(message.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render a slice of diagnostics to **stderr** using the given
/// [`SourceMap`] as the lookup table for file names and source text,
/// and the given [`ExpansionTable`] to resolve macro backtrace
/// information for tokens that came from an expansion.
///
/// ANSI colour codes are emitted only when stderr is attached to a
/// terminal.
pub fn render_diagnostics(
    source_map: &SourceMap,
    expansions: &ExpansionTable,
    diagnostics: &[Diagnostic],
) {
    let use_color = std::io::stderr().is_terminal();
    for diag in diagnostics {
        write_report(
            diag,
            source_map,
            expansions,
            &mut std::io::stderr(),
            use_color,
        )
        .expect("failed to write diagnostic to stderr");
    }
}

/// Render a slice of diagnostics to a [`String`] without ANSI colour codes.
pub fn render_diagnostics_to_string(
    source_map: &SourceMap,
    expansions: &ExpansionTable,
    diagnostics: &[Diagnostic],
) -> String {
    let mut buf: Vec<u8> = Vec::new();
    for diag in diagnostics {
        write_report(diag, source_map, expansions, &mut buf, false)
            .expect("failed to write diagnostic to buffer");
    }
    String::from_utf8(buf).unwrap_or_default()
}

/// ariadne `Cache` adapter over a `SourceMap`, keyed by `FileId`.
struct SourceMapCache<'a> {
    map: &'a SourceMap,
    sources: std::collections::HashMap<FileId, Source<String>>,
}

impl<'a> SourceMapCache<'a> {
    fn new(map: &'a SourceMap) -> Self {
        Self {
            map,
            sources: std::collections::HashMap::new(),
        }
    }
}

impl Cache<FileId> for SourceMapCache<'_> {
    type Storage = String;

    fn fetch(&mut self, id: &FileId) -> Result<&Source<String>, Box<dyn fmt::Debug + '_>> {
        let src = self
            .map
            .get(*id)
            .map(|sf| sf.source.clone())
            .unwrap_or_default();
        let entry = self.sources.entry(*id).or_insert_with(|| Source::from(src));
        Ok(entry)
    }

    fn display<'b>(&self, id: &'b FileId) -> Option<Box<dyn fmt::Display + 'b>> {
        let name = self
            .map
            .get(*id)
            .map_or_else(|| "<unknown>".to_string(), |sf| sf.name.clone());
        Some(Box::new(name))
    }
}

/// Core rendering helper shared by the public render functions.
///
/// When `diag.span.expanded_from` names an expansion frame, the frame's
/// invocation span (and every ancestor frame along the parent chain) is
/// rendered as an auxiliary label prefixed with `in expansion of macro
/// 'M'`.  The primary error label stays on the token's own span so the
/// user still sees exactly which token tripped the check.
fn write_report(
    diag: &Diagnostic,
    source_map: &SourceMap,
    expansions: &ExpansionTable,
    writer: &mut dyn std::io::Write,
    color: bool,
) -> std::io::Result<()> {
    let kind = match diag.severity {
        Severity::Error => ReportKind::Error,
        Severity::Warning => ReportKind::Warning,
        Severity::Note => ReportKind::Advice,
    };

    let primary_color = match diag.severity {
        Severity::Error => Color::Red,
        Severity::Warning => Color::Yellow,
        Severity::Note => Color::Blue,
    };

    let mut builder = Report::build(kind, diag.span.file, diag.span.start as usize)
        .with_config(Config::default().with_color(color))
        .with_message(&diag.message)
        .with_label(
            AriadneLabel::new((diag.span.file, diag.span.range()))
                .with_message(&diag.message)
                .with_color(primary_color),
        );

    for (i, label) in diag.labels.iter().enumerate() {
        let label_color = if i == 0 { primary_color } else { Color::Cyan };
        builder = builder.with_label(
            AriadneLabel::new((label.span.file, label.span.range()))
                .with_message(&label.message)
                .with_color(label_color),
        );
    }

    // If the primary span was produced by a macro expansion, attach an
    // "in expansion of macro 'M'" label for each frame in the chain.
    // Innermost frame first — ariadne renders labels in the order the
    // user sees them in the source, but the messages themselves make
    // the chain order explicit.
    for frame in expansions.backtrace(diag.span.expanded_from) {
        let msg = format!("in expansion of macro '{}'", frame.macro_name);
        builder = builder.with_label(
            AriadneLabel::new((frame.invocation_span.file, frame.invocation_span.range()))
                .with_message(msg)
                .with_color(Color::Cyan),
        );
    }

    for note in &diag.notes {
        builder = builder.with_note(note);
    }

    let mut cache = SourceMapCache::new(source_map);
    builder.finish().write(&mut cache, writer)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
