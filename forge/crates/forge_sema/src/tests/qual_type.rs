//! Qualifier bookkeeping on [`QualType`].
//!
//! Sema wraps every `Type` in a [`QualType`] so that qualifiers travel
//! alongside the shape.  These tests lock in the small set of helpers
//! callers lean on every day.

use super::helpers::*;
use crate::types::{QualType, Type};

#[test]
fn unqualified_has_no_qualifiers() {
    let qt = QualType::unqualified(int());
    assert!(!qt.is_const);
    assert!(!qt.is_volatile);
    assert!(!qt.is_restrict);
    assert!(!qt.is_atomic);
    assert_eq!(qt.explicit_align, None);
    assert!(!qt.has_any_qualifier());
}

#[test]
fn with_const_sets_only_const() {
    let qt = QualType::unqualified(int()).with_const();
    assert!(qt.is_const);
    assert!(!qt.is_volatile);
    assert!(!qt.is_restrict);
    assert!(!qt.is_atomic);
    assert!(qt.has_any_qualifier());
}

#[test]
fn strip_qualifiers_returns_inner_type() {
    let mut qt = QualType::unqualified(int());
    qt.is_const = true;
    qt.is_volatile = true;

    assert_eq!(qt.strip_qualifiers(), int());
}

#[test]
fn has_any_qualifier_detects_each_flag() {
    let mut qt = QualType::unqualified(int());
    assert!(!qt.has_any_qualifier());

    qt.is_const = true;
    assert!(qt.has_any_qualifier());

    qt.is_const = false;
    qt.is_volatile = true;
    assert!(qt.has_any_qualifier());

    qt.is_volatile = false;
    qt.is_restrict = true;
    assert!(qt.has_any_qualifier());

    qt.is_restrict = false;
    qt.is_atomic = true;
    assert!(qt.has_any_qualifier());
}

#[test]
fn explicit_align_is_not_a_qualifier() {
    let mut qt = QualType::unqualified(int());
    qt.explicit_align = Some(16);
    assert!(!qt.has_any_qualifier());
}

#[test]
fn qualtype_equality_is_structural() {
    let a = QualType::unqualified(Type::Int { is_unsigned: false });
    let b = QualType::unqualified(Type::Int { is_unsigned: false });
    assert_eq!(a, b);

    let c = QualType::unqualified(Type::Int { is_unsigned: true });
    assert_ne!(a, c);
}
