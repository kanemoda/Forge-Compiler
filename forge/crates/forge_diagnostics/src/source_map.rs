//! Source file registry and byte-offset/line mapping.
//!
//! A [`SourceMap`] owns every source file read by the compiler — the
//! translation-unit root plus every `#include`d header.  Each file is
//! handed a fresh [`FileId`] that callers thread through every
//! [`crate::Diagnostic`] so multi-file diagnostics can look up the
//! file's name and text when rendering.
//!
//! [`SourceFile::line_col`] converts a byte offset into a 1-based
//! `(line, column)` pair using a pre-computed `line_starts` table.
//!
//! This module is intentionally dead-code plumbing: the rest of the
//! pipeline does not yet thread `FileId` into [`crate::Diagnostic`].
//! Later sub-prompts of the Phase 2 multi-file Span fix will connect it.

/// Stable identifier for a source file registered in a [`SourceMap`].
///
/// Allocated sequentially starting at [`FileId::PRIMARY`] (= 0).  The
/// wrapping `u32` limits a single compilation to `u32::MAX - 1` distinct
/// source files, far beyond any real-world translation unit.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct FileId(pub u32);

impl FileId {
    /// Sentinel meaning "no file".  Used by spans constructed before a
    /// [`SourceMap`] is available or for compiler-synthesised origins.
    pub const INVALID: FileId = FileId(u32::MAX);

    /// The translation-unit root — always the first file handed to a
    /// fresh [`SourceMap`].
    pub const PRIMARY: FileId = FileId(0);
}

/// A single registered source file with its contents and a line-start
/// index for fast `offset → (line, column)` conversion.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// The [`FileId`] that identifies this file inside its [`SourceMap`].
    pub id: FileId,
    /// Display name (`"main.c"`, `"/usr/include/stdio.h"`, …).
    pub name: String,
    /// The full file contents.
    pub source: String,
    /// Byte offset of each line's start.
    ///
    /// `line_starts[0]` is always `0`.  A new entry is appended for the
    /// byte *after* every `\n`, so `line_starts.len()` equals the file's
    /// line count (an empty file has one line).
    pub line_starts: Vec<u32>,
}

impl SourceFile {
    /// Build a new [`SourceFile`] and pre-compute its `line_starts` table.
    ///
    /// `\r\n` is treated as a single line break (the `\n` is what starts
    /// the new line) and an empty source yields `line_starts == vec![0]`.
    pub fn new(id: FileId, name: String, source: String) -> Self {
        let mut line_starts =
            Vec::with_capacity(source.bytes().filter(|b| *b == b'\n').count() + 1);
        line_starts.push(0u32);
        for (i, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        Self {
            id,
            name,
            source,
            line_starts,
        }
    }

    /// Convert a byte offset into a 1-based `(line, column)` pair.
    ///
    /// Columns are measured in *bytes* from the start of the line, not
    /// Unicode grapheme clusters.  Offsets past end-of-file saturate to
    /// the last valid position rather than panicking — callers that
    /// carry stale or synthetic spans do not take the compiler down.
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        let clamped = offset.min(self.source.len() as u32);
        let line_index = match self.line_starts.binary_search(&clamped) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = self.line_starts[line_index];
        (line_index as u32 + 1, clamped - line_start + 1)
    }
}

/// Registry of every source file known to the compiler.
///
/// Files are added via [`SourceMap::add_file`] in the order the driver
/// encounters them; [`FileId`]s are handed out sequentially starting at
/// [`FileId::PRIMARY`].
#[derive(Debug, Clone)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    /// Construct an empty source map.
    pub fn new() -> Self {
        Self { files: Vec::new() }
    }

    /// Register a new source file and return its freshly-allocated
    /// [`FileId`].
    ///
    /// IDs are allocated sequentially, so the first call returns
    /// [`FileId::PRIMARY`].
    pub fn add_file(&mut self, name: String, source: String) -> FileId {
        let id = FileId(self.files.len() as u32);
        self.files.push(SourceFile::new(id, name, source));
        id
    }

    /// Look up a source file by its [`FileId`], returning `None` for
    /// unknown ids or for [`FileId::INVALID`].
    pub fn get(&self, id: FileId) -> Option<&SourceFile> {
        self.files.get(id.0 as usize)
    }

    /// Look up a source file that the caller guarantees is valid.
    ///
    /// Panics on [`FileId::INVALID`] or any id past the end of the map.
    /// Prefer [`SourceMap::get`] on paths where the id may be invalid
    /// (for example, spans carried through from a pre-SourceMap phase).
    pub fn get_or_panic(&self, id: FileId) -> &SourceFile {
        match self.get(id) {
            Some(file) => file,
            None => panic!("FileId {id:?} not present in SourceMap"),
        }
    }

    /// Number of files currently registered.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether no files are registered yet.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Iterate over every registered file in the order they were added.
    pub fn iter(&self) -> impl Iterator<Item = &SourceFile> {
        self.files.iter()
    }
}

impl Default for SourceMap {
    fn default() -> Self {
        Self::new()
    }
}
