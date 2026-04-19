//! C17 §6.3.1.8 usual arithmetic conversion tests.
//!
//! The UAC is the hot path for every binary arithmetic operator.  The
//! table below walks every step of the standard algorithm on LP64:
//!
//! 1. any `long double` → `long double`
//! 2. any `double` → `double`
//! 3. any `float` → `float`
//! 4. integer promote both sides
//! 5. same type → that type
//! 6. same signedness → higher rank
//! 7. unsigned rank ≥ signed rank → unsigned
//! 8. signed range ⊇ unsigned range → signed
//! 9. otherwise → unsigned version of signed type

use super::helpers::*;
use crate::types::usual_arithmetic_conversions;

// ---------- arithmetic type hierarchy (steps 1–3) ----------

#[test]
fn any_long_double_wins() {
    let t = ti();
    assert_eq!(
        usual_arithmetic_conversions(&long_double(), &int(), &t),
        long_double()
    );
    assert_eq!(
        usual_arithmetic_conversions(&int(), &long_double(), &t),
        long_double()
    );
    assert_eq!(
        usual_arithmetic_conversions(&long_double(), &t_double(), &t),
        long_double()
    );
}

#[test]
fn any_double_wins_over_float_and_int() {
    let t = ti();
    assert_eq!(
        usual_arithmetic_conversions(&t_double(), &int(), &t),
        t_double()
    );
    assert_eq!(
        usual_arithmetic_conversions(&t_float(), &t_double(), &t),
        t_double()
    );
}

#[test]
fn any_float_wins_over_int() {
    let t = ti();
    assert_eq!(
        usual_arithmetic_conversions(&t_float(), &int(), &t),
        t_float()
    );
    assert_eq!(
        usual_arithmetic_conversions(&int(), &t_float(), &t),
        t_float()
    );
}

// ---------- step 4: both operands promote before integer rules ----------

#[test]
fn small_ints_promote_before_balancing() {
    let t = ti();
    // `char + char` — both promote to int, then step 5 applies.
    assert_eq!(
        usual_arithmetic_conversions(&char_plain(), &char_plain(), &t),
        int()
    );
    assert_eq!(
        usual_arithmetic_conversions(&short(), &char_plain(), &t),
        int()
    );
    assert_eq!(
        usual_arithmetic_conversions(&t_bool(), &ushort(), &t),
        int()
    );
}

// ---------- step 5: identical after promotion ----------

#[test]
fn int_plus_int_is_int() {
    assert_eq!(usual_arithmetic_conversions(&int(), &int(), &ti()), int());
}

#[test]
fn long_plus_long_is_long() {
    assert_eq!(
        usual_arithmetic_conversions(&long(), &long(), &ti()),
        long()
    );
}

// ---------- step 6: same signedness, different rank ----------

#[test]
fn int_plus_long_is_long() {
    assert_eq!(usual_arithmetic_conversions(&int(), &long(), &ti()), long());
    assert_eq!(usual_arithmetic_conversions(&long(), &int(), &ti()), long());
}

#[test]
fn long_plus_long_long_is_long_long() {
    assert_eq!(
        usual_arithmetic_conversions(&long(), &llong(), &ti()),
        llong()
    );
}

#[test]
fn uint_plus_ulong_is_ulong() {
    assert_eq!(
        usual_arithmetic_conversions(&uint(), &ulong(), &ti()),
        ulong()
    );
}

// ---------- step 7: unsigned rank ≥ signed rank → unsigned wins ----------

#[test]
fn int_plus_uint_is_uint() {
    assert_eq!(usual_arithmetic_conversions(&int(), &uint(), &ti()), uint());
    assert_eq!(usual_arithmetic_conversions(&uint(), &int(), &ti()), uint());
}

#[test]
fn unsigned_long_long_beats_signed_long_long() {
    assert_eq!(
        usual_arithmetic_conversions(&llong(), &ullong(), &ti()),
        ullong()
    );
}

#[test]
fn int_plus_ullong_is_ullong() {
    assert_eq!(
        usual_arithmetic_conversions(&int(), &ullong(), &ti()),
        ullong()
    );
}

// ---------- step 8: signed strictly wider → signed wins ----------

#[test]
fn long_plus_uint_is_long_on_lp64() {
    // LP64: long is 64-bit, unsigned int is 32-bit → signed long can
    // represent every unsigned int value.
    assert_eq!(
        usual_arithmetic_conversions(&long(), &uint(), &ti()),
        long()
    );
    assert_eq!(
        usual_arithmetic_conversions(&uint(), &long(), &ti()),
        long()
    );
}

#[test]
fn long_long_plus_uint_is_long_long_on_lp64() {
    assert_eq!(
        usual_arithmetic_conversions(&llong(), &uint(), &ti()),
        llong()
    );
}

// ---------- step 9: fall back to unsigned version of signed type ----------

#[test]
fn unsigned_long_plus_long_is_unsigned_long_on_lp64() {
    // Both 64 bits wide — signed cannot represent `unsigned long`'s
    // range, so fall through to the unsigned version of the signed type.
    assert_eq!(
        usual_arithmetic_conversions(&ulong(), &long(), &ti()),
        ulong()
    );
    assert_eq!(
        usual_arithmetic_conversions(&long(), &ulong(), &ti()),
        ulong()
    );
}

// ---------- sanity: integer_promotion is called before int rules ----------

#[test]
fn ushort_plus_ushort_is_int_on_lp64() {
    // Two unsigned shorts promote to signed int (LP64), which gives int.
    assert_eq!(
        usual_arithmetic_conversions(&ushort(), &ushort(), &ti()),
        int()
    );
}
