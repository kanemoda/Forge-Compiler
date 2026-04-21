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
