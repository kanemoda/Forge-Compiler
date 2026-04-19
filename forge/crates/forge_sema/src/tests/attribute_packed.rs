//! Acceptance tests for `__attribute__((packed))`.
//!
//! # Phase 5 note
//!
//! Sema currently **parses and ignores** `__attribute__((packed))` —
//! `StructLayout::is_packed` stays `false` and natural alignment still
//! drives member offsets.  Wiring the attribute into layout is a
//! Phase 5 item.  These tests pin the acceptance contract (packed
//! declarations do not error) and the current layout as a regression
//! guard for the pre-packed baseline.

use super::helpers::assert_source_clean;

#[test]
fn packed_on_struct_is_accepted() {
    assert_source_clean(
        "struct __attribute__((packed)) P {
             char a;
             int b;
         };
         int main(void) {
             struct P p;
             p.a = 0;
             p.b = 0;
             return p.b;
         }",
    );
}

#[test]
fn packed_on_individual_member_is_accepted() {
    // glibc's struct _IO_FILE scatters packed attributes on individual
    // fields — sema must accept that shape without complaint.
    assert_source_clean(
        "struct P {
             char a;
             int b __attribute__((packed));
         };
         int main(void) {
             struct P p;
             p.a = 0;
             p.b = 0;
             return p.b;
         }",
    );
}

#[test]
fn unpacked_baseline_keeps_natural_alignment() {
    // Regression guard: without the packed attribute, sizeof(struct { char;
    // int; }) is 8 on LP64, not 5.  When packed-layout support lands, the
    // packed test should check 5 and this test must remain at 8.
    assert_source_clean(
        "struct U { char a; int b; };
         _Static_assert(sizeof(struct U) == 8, \"unpacked size must be 8\");
         _Static_assert(_Alignof(struct U) == 4, \"unpacked align must be 4\");
         int main(void) { return 0; }",
    );
}

#[test]
fn packed_structs_still_allow_member_access() {
    // Member access must still work on a packed struct — the attribute
    // is a layout hint, not a visibility one.
    assert_source_clean(
        "struct __attribute__((packed)) P {
             char c;
             int i;
         };
         int read_packed(void) {
             struct P p;
             p.c = 1;
             p.i = 2;
             return p.c + p.i;
         }
         int main(void) { return read_packed(); }",
    );
}
