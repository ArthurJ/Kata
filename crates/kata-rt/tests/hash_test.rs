use kata_rt::{kata_rt_hash_int, kata_rt_hash_rational, kata_rt_hash_text};
use std::ffi::CString;

// ── Helpers ─────────────────────────────────────────────────

/// SMI-tag um i64: `(val << 1) | 1`.
fn tag_smi(val: i64) -> i64 {
    (val << 1) | 1
}

// ── Int ─────────────────────────────────────────────────────

#[test]
fn hash_int_deterministic() {
    let smi = tag_smi(42);
    let h1 = kata_rt_hash_int(smi);
    let h2 = kata_rt_hash_int(smi);
    assert_eq!(h1, h2, "same SMI must produce same hash");
}

#[test]
fn hash_int_different_values_different_hash() {
    let h_a = kata_rt_hash_int(tag_smi(1));
    let h_b = kata_rt_hash_int(tag_smi(2));
    assert_ne!(h_a, h_b, "different SMI values must hash differently");
}

#[test]
fn hash_int_zero() {
    let h0 = kata_rt_hash_int(tag_smi(0));
    // Non-zero hash expected (FNV_OFFSET is non-zero, then mixed)
    assert_ne!(h0, 0);
}

#[test]
fn hash_int_negative() {
    let h = kata_rt_hash_int(tag_smi(-100));
    let h2 = kata_rt_hash_int(tag_smi(-100));
    assert_eq!(h, h2, "negative SMI must be deterministic");
}

// ── Text ────────────────────────────────────────────────────

#[test]
fn hash_text_deterministic() {
    // KEY TEST: same content at different addresses must produce same hash.
    let s1 = CString::new("hello").unwrap();
    let s2 = CString::new("hello").unwrap();
    assert_ne!(s1.as_ptr(), s2.as_ptr(), "precondition: different addresses");

    let h1 = kata_rt_hash_text(s1.as_ptr() as i64);
    let h2 = kata_rt_hash_text(s2.as_ptr() as i64);
    assert_eq!(
        h1, h2,
        "content-based hash: same text at different addresses must match"
    );
}

#[test]
fn hash_text_different_content_different_hash() {
    let s1 = CString::new("hello").unwrap();
    let s2 = CString::new("world").unwrap();

    let h1 = kata_rt_hash_text(s1.as_ptr() as i64);
    let h2 = kata_rt_hash_text(s2.as_ptr() as i64);
    assert_ne!(h1, h2, "different text content must hash differently");
}

#[test]
fn hash_text_null_returns_offset() {
    let h = kata_rt_hash_text(0);
    // FNV_OFFSET = 0xcbf29ce484222325
    assert_eq!(h as u64, 0xcbf29ce484222325, "null pointer must return FNV_OFFSET");
}

// ── Rational ────────────────────────────────────────────────

#[test]
fn hash_rational_deterministic() {
    // Allocate a simple rational struct: numer=i64, denom=i64
    let numer: i64 = 1;
    let denom: i64 = 2;
    // Stack layout — place numer then denom consecutively.
    let layout: [i64; 2] = [numer, denom];
    let ptr = &layout as *const [i64; 2] as i64;

    let h1 = kata_rt_hash_rational(ptr);
    let h2 = kata_rt_hash_rational(ptr);
    assert_eq!(h1, h2, "same rational must produce same hash");
}

#[test]
fn hash_rational_null_returns_offset() {
    let h = kata_rt_hash_rational(0);
    assert_eq!(h as u64, 0xcbf29ce484222325, "null pointer must return FNV_OFFSET");
}