//! Tests for [`crate::source_map`] — [`FileId`], [`SourceFile`],
//! [`SourceMap`].

use crate::{FileId, SourceFile, SourceMap};

#[test]
fn fileid_invalid_constant_has_max_value() {
    assert_eq!(FileId::INVALID, FileId(u32::MAX));
}

#[test]
fn fileid_primary_constant_is_zero() {
    assert_eq!(FileId::PRIMARY, FileId(0));
}

#[test]
fn add_file_returns_sequential_ids() {
    let mut sm = SourceMap::new();
    let a = sm.add_file("a.c".into(), String::new());
    let b = sm.add_file("b.c".into(), String::new());
    let c = sm.add_file("c.c".into(), String::new());
    assert_eq!(a, FileId(0));
    assert_eq!(b, FileId(1));
    assert_eq!(c, FileId(2));
    assert_eq!(a, FileId::PRIMARY);
}

#[test]
fn source_file_line_col_offset_zero_is_line_one_col_one() {
    let file = SourceFile::new(FileId::PRIMARY, "empty.c".into(), String::new());
    assert_eq!(file.line_starts, vec![0]);
    assert_eq!(file.line_col(0), (1, 1));
}

#[test]
fn source_file_line_col_first_line_middle() {
    let file = SourceFile::new(FileId::PRIMARY, "f.c".into(), "hello\nworld".into());
    // h=0, e=1, l=2, l=3 → line 1, column 4 (1-based).
    assert_eq!(file.line_col(3), (1, 4));
}

#[test]
fn source_file_line_col_second_line_start() {
    let file = SourceFile::new(FileId::PRIMARY, "f.c".into(), "hello\nworld".into());
    // '\n' at byte 5; byte 6 is the first byte of line 2.
    assert_eq!(file.line_col(6), (2, 1));
}

#[test]
fn source_file_line_col_crlf_handled() {
    // "\r\n" is a single logical break: \n at byte 4 starts line 2 at byte 5.
    let file = SourceFile::new(FileId::PRIMARY, "f.c".into(), "foo\r\nbar".into());
    assert_eq!(file.line_col(5), (2, 1));
}

#[test]
fn source_file_line_col_past_eof_saturates() {
    let file = SourceFile::new(FileId::PRIMARY, "f.c".into(), "hi".into());
    // Must not panic; saturates to byte len() of the final line.
    let (line, col) = file.line_col(1000);
    assert_eq!(line, 1);
    assert_eq!(col, 3); // one past 'i', the last valid byte-column.
}

#[test]
fn source_map_get_invalid_returns_none() {
    let sm = SourceMap::new();
    assert!(sm.get(FileId::INVALID).is_none());
    assert!(sm.get(FileId(42)).is_none());
}

#[test]
fn source_map_len_grows_with_add_file() {
    let mut sm = SourceMap::new();
    assert_eq!(sm.len(), 0);
    assert!(sm.is_empty());
    sm.add_file("a.c".into(), String::new());
    assert_eq!(sm.len(), 1);
    assert!(!sm.is_empty());
    sm.add_file("b.c".into(), String::new());
    assert_eq!(sm.len(), 2);
}

#[test]
fn source_map_iter_yields_in_insertion_order() {
    let mut sm = SourceMap::new();
    sm.add_file("one.c".into(), "1".into());
    sm.add_file("two.c".into(), "2".into());
    sm.add_file("three.c".into(), "3".into());
    let names: Vec<&str> = sm.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["one.c", "two.c", "three.c"]);
}

// -----------------------------------------------------------------------
// Phase 2 wrap-up — explicit edge-case coverage for `line_starts` /
// `line_col`.  Some of these overlap with earlier tests; they are kept
// as stand-alone named assertions so a regression points at the exact
// invariant that broke.
// -----------------------------------------------------------------------

/// A file that does not end with `\n` still has every line reachable —
/// the last line's bytes sit between the final `line_starts` entry and
/// `source.len()`.
#[test]
fn source_file_no_trailing_newline_last_line_reachable() {
    let file = SourceFile::new(FileId::PRIMARY, "f.c".into(), "a\nb\nc".into());
    // Three logical lines: "a", "b", "c" — no trailing newline after "c".
    assert_eq!(file.line_starts, vec![0, 2, 4]);
    // The final byte 'c' sits at offset 4, which is the start of line 3.
    assert_eq!(file.line_col(4), (3, 1));
    // One past EOF (5) saturates to "just after 'c'" still on line 3.
    assert_eq!(file.line_col(5), (3, 2));
}

/// Columns are measured in **bytes**, not Unicode grapheme clusters — a
/// 2-byte UTF-8 character advances the column counter by 2.  This is
/// documented on [`SourceFile::line_col`] and enforced here.
#[test]
fn source_file_utf8_multibyte_column_is_byte_offset_not_grapheme() {
    // "é" is U+00E9, two bytes in UTF-8 (0xC3 0xA9).  After it, the
    // next character 'x' sits at byte offset 2, column 3 (1-based).
    let file = SourceFile::new(FileId::PRIMARY, "f.c".into(), "éx".into());
    assert_eq!(file.line_col(0), (1, 1)); // first byte of 'é'
    assert_eq!(file.line_col(2), (1, 3)); // 'x' — col 3, not col 2
}

/// A leading UTF-8 BOM (`0xEF 0xBB 0xBF`) is NOT stripped for v1 —
/// it sits at byte offsets 0..3 of line 1 and contributes to the
/// column count.  Documented here so a behaviour change is caught.
#[test]
fn source_file_bom_at_start_is_retained_in_column_indexing() {
    let source = format!("{}int x;", '\u{FEFF}');
    let file = SourceFile::new(FileId::PRIMARY, "f.c".into(), source);
    // The BOM is three bytes, so 'i' is at byte offset 3, column 4.
    assert_eq!(file.line_col(3), (1, 4));
}

/// Very long single lines must not overflow `u32` arithmetic inside
/// `line_col` — we do not support sources larger than `u32::MAX` bytes,
/// but `line_col` itself should handle long lines cleanly up to that cap.
#[test]
fn source_file_very_long_line_does_not_overflow_u32() {
    // 100_000 bytes on one line — well below u32 but far past any
    // pathological real-world shape.
    let source = "x".repeat(100_000);
    let file = SourceFile::new(FileId::PRIMARY, "f.c".into(), source);
    assert_eq!(file.line_starts, vec![0]);
    // Last valid byte is at offset 99_999 on line 1, col 100_000 (1-based).
    assert_eq!(file.line_col(99_999), (1, 100_000));
}
