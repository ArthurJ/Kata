use kata_rt::{
    decode_smi_pub, encode_smi_pub, fits_smi_pub, is_smi_pub, tag_int_from_str, tag_int_pub,
};

// ── SMI tagging básico ──────────────────────────────────────

#[test]
fn small_int_is_smi() {
    let tagged = tag_int_pub(42);
    assert!(is_smi_pub(tagged), "42 deve ser SMI");
}

#[test]
fn zero_is_smi() {
    let tagged = tag_int_pub(0);
    assert!(is_smi_pub(tagged));
}

#[test]
fn negative_small_is_smi() {
    let tagged = tag_int_pub(-42);
    assert!(is_smi_pub(tagged));
}

#[test]
fn large_int_is_not_smi() {
    // Acima do limite SMI (2^62 - 1) deve ir para heap
    let large = (1i128 << 62) as i64; // 2^62, fora do range SMI
    let tagged = tag_int_pub(large);
    assert!(!is_smi_pub(tagged), "2^62 não deve ser SMI");
}

// ── Roundtrip encode/decode ──────────────────────────────────

#[test]
fn smi_roundtrip_positive() {
    let tagged = encode_smi_pub(42);
    assert_eq!(decode_smi_pub(tagged), 42);
}

#[test]
fn smi_roundtrip_zero() {
    let tagged = encode_smi_pub(0);
    assert_eq!(decode_smi_pub(tagged), 0);
}

#[test]
fn smi_roundtrip_negative() {
    let tagged = encode_smi_pub(-100);
    assert_eq!(decode_smi_pub(tagged), -100);
}

// ── fits_smi ─────────────────────────────────────────────────

#[test]
fn fits_smi_small() {
    assert!(fits_smi_pub(42));
    assert!(fits_smi_pub(0));
    assert!(fits_smi_pub(-42));
}

#[test]
fn fits_smi_boundary() {
    // SMI_MAX = 2^62 - 1
    assert!(fits_smi_pub((1i64 << 62) - 1));
    // SMI_MIN = -2^62
    assert!(fits_smi_pub(-(1i64 << 62)));
}

#[test]
fn does_not_fit_smi_large() {
    assert!(!fits_smi_pub(1i64 << 62));
    assert!(!fits_smi_pub(-(1i64 << 62) - 1));
}

// ── Aritmética SMI+SMI → SMI ──────────────────────────────────

#[test]
fn bi_add_smi_smi() {
    let a = tag_int_pub(1);
    let b = tag_int_pub(2);
    let result = unsafe { kata_rt_bi_add(a, b) };
    assert!(is_smi_pub(result));
    assert_eq!(decode_smi_pub(result), 3);
}

#[test]
fn bi_sub_smi_smi() {
    let a = tag_int_pub(10);
    let b = tag_int_pub(3);
    let result = unsafe { kata_rt_bi_sub(a, b) };
    assert_eq!(decode_smi_pub(result), 7);
}

#[test]
fn bi_mul_smi_smi() {
    let a = tag_int_pub(6);
    let b = tag_int_pub(7);
    let result = unsafe { kata_rt_bi_mul(a, b) };
    assert_eq!(decode_smi_pub(result), 42);
}

// ── Aritmética SMI+SMI → BigInt (overflow) ───────────────────

#[test]
fn bi_add_overflow_promotes_to_bigint() {
    let a = tag_int_pub((1i64 << 61));
    let b = tag_int_pub((1i64 << 61));
    let result = unsafe { kata_rt_bi_add(a, b) };
    // 2^61 + 2^61 = 2^62 que não cabe em SMI
    assert!(!is_smi_pub(result), "overflow deve promover para BigInt");
    let s = kata_rt::bigint_to_string(result);
    assert_eq!(s, "4611686018427387904"); // 2^62
}

// ── BigInt grande ─────────────────────────────────────────────

#[test]
fn bi_mul_bigint_no_overflow() {
    // 99999999999999999999 * 99999999999999999999
    let a = tag_int_from_str("99999999999999999999");
    let b = tag_int_from_str("99999999999999999999");
    let result = unsafe { kata_rt_bi_mul(a, b) };
    assert!(!is_smi_pub(result));
    let s = kata_rt::bigint_to_string(result);
    assert_eq!(s, "9999999999999999999800000000000000000001");
}

// ── Comparação ───────────────────────────────────────────────

#[test]
fn bi_eq_returns_true() {
    let a = tag_int_pub(42);
    let b = tag_int_pub(42);
    assert_eq!(unsafe { kata_rt_bi_eq(a, b) }, 1);
}

#[test]
fn bi_eq_returns_false() {
    let a = tag_int_pub(42);
    let b = tag_int_pub(43);
    assert_eq!(unsafe { kata_rt_bi_eq(a, b) }, 0);
}

#[test]
fn bi_lt_returns_true() {
    let a = tag_int_pub(1);
    let b = tag_int_pub(2);
    assert_eq!(unsafe { kata_rt_bi_lt(a, b) }, 1);
}

#[test]
fn bi_lt_returns_false() {
    let a = tag_int_pub(2);
    let b = tag_int_pub(1);
    assert_eq!(unsafe { kata_rt_bi_lt(a, b) }, 0);
}

#[test]
fn bi_gt_returns_true() {
    let a = tag_int_pub(5);
    let b = tag_int_pub(3);
    assert_eq!(unsafe { kata_rt_bi_gt(a, b) }, 1);
}

// ── Literais de diferentes bases ─────────────────────────────

#[test]
fn tag_int_from_hex() {
    let tagged = tag_int_from_str("0xFF");
    assert_eq!(decode_smi_pub(tagged), 255);
}

#[test]
fn tag_int_from_oct() {
    let tagged = tag_int_from_str("0o77");
    assert_eq!(decode_smi_pub(tagged), 63);
}

#[test]
fn tag_int_from_bin() {
    let tagged = tag_int_from_str("0b1010");
    assert_eq!(decode_smi_pub(tagged), 10);
}

#[test]
fn tag_int_with_underscore_separator() {
    let tagged = tag_int_from_str("1_000");
    assert_eq!(decode_smi_pub(tagged), 1000);
}

#[test]
fn tag_int_from_decimal_explicit() {
    let tagged = tag_int_from_str("0d42");
    assert_eq!(decode_smi_pub(tagged), 42);
}

#[test]
fn tag_int_from_decimal_explicit_large() {
    let tagged = tag_int_from_str("0d99999999999999999999");
    assert!(!is_smi_pub(tagged));
    assert_eq!(kata_rt::bigint_to_string(tagged), "99999999999999999999");
}

#[test]
fn tag_int_from_hex_uppercase_prefix() {
    let tagged = tag_int_from_str("0XFF");
    assert_eq!(decode_smi_pub(tagged), 255);
}

#[test]
fn tag_int_from_oct_uppercase_prefix() {
    let tagged = tag_int_from_str("0O77");
    assert_eq!(decode_smi_pub(tagged), 63);
}

#[test]
fn tag_int_from_bin_uppercase_prefix() {
    let tagged = tag_int_from_str("0B1010");
    assert_eq!(decode_smi_pub(tagged), 10);
}

#[test]
fn tag_int_from_decimal_explicit_uppercase_prefix() {
    let tagged = tag_int_from_str("0D42");
    assert_eq!(decode_smi_pub(tagged), 42);
}

#[test]
fn tag_int_hex_with_underscore() {
    let tagged = tag_int_from_str("0xFF_FF");
    assert_eq!(decode_smi_pub(tagged), 65535);
}

#[test]
fn tag_int_bin_with_underscore() {
    let tagged = tag_int_from_str("0b1010_1010");
    assert_eq!(decode_smi_pub(tagged), 170);
}

#[test]
fn tag_int_oct_with_underscore() {
    let tagged = tag_int_from_str("0o777_777");
    assert_eq!(decode_smi_pub(tagged), 262143);
}

#[test]
fn tag_int_decimal_with_multiple_underscores() {
    let tagged = tag_int_from_str("1_000_000");
    assert_eq!(decode_smi_pub(tagged), 1000000);
}

#[test]
fn tag_int_decimal_explicit_with_underscore() {
    let tagged = tag_int_from_str("0d1_000");
    assert_eq!(decode_smi_pub(tagged), 1000);
}

#[test]
fn tag_int_zero() {
    let tagged = tag_int_from_str("0");
    assert_eq!(decode_smi_pub(tagged), 0);
}

#[test]
fn tag_int_hex_zero() {
    let tagged = tag_int_from_str("0x0");
    assert_eq!(decode_smi_pub(tagged), 0);
}

#[test]
fn tag_int_negative_decimal() {
    let tagged = tag_int_from_str("-42");
    assert_eq!(decode_smi_pub(tagged), -42);
}

#[test]
fn tag_int_negative_hex() {
    let tagged = tag_int_from_str("-0xFF");
    assert_eq!(decode_smi_pub(tagged), -255);
}

#[test]
fn tag_int_negative_oct() {
    let tagged = tag_int_from_str("-0o77");
    assert_eq!(decode_smi_pub(tagged), -63);
}

#[test]
fn tag_int_negative_bin() {
    let tagged = tag_int_from_str("-0b1010");
    assert_eq!(decode_smi_pub(tagged), -10);
}

#[test]
fn tag_int_negative_decimal_explicit() {
    let tagged = tag_int_from_str("-0d42");
    assert_eq!(decode_smi_pub(tagged), -42);
}

#[test]
fn tag_int_negative_hex_uppercase_prefix() {
    let tagged = tag_int_from_str("-0XFF");
    assert_eq!(decode_smi_pub(tagged), -255);
}

#[test]
fn tag_int_negative_bigint() {
    let tagged = tag_int_from_str("-99999999999999999999");
    assert!(!is_smi_pub(tagged));
    assert_eq!(kata_rt::bigint_to_string(tagged), "-99999999999999999999");
}

#[test]
fn tag_int_hex_lowercase_digits() {
    let tagged = tag_int_from_str("0xdeadbeef");
    assert_eq!(decode_smi_pub(tagged), 0xdeadbeef);
}

#[test]
fn tag_int_hex_uppercase_digits() {
    let tagged = tag_int_from_str("0xDEADBEEF");
    assert_eq!(decode_smi_pub(tagged), 0xDEADBEEF);
}

#[test]
fn tag_int_hex_mixed_case_digits() {
    let tagged = tag_int_from_str("0xDeAdBeEf");
    assert_eq!(decode_smi_pub(tagged), 0xDEADBEEF);
}

#[test]
fn tag_int_bigint_from_hex() {
    let tagged = tag_int_from_str("0xDEADBEEF1234567890");
    assert!(!is_smi_pub(tagged));
    assert_eq!(kata_rt::bigint_to_string(tagged), "4107696891239123548304");
}

// ── show ─────────────────────────────────────────────────────

#[test]
fn show_smi() {
    let tagged = tag_int_pub(42);
    assert_eq!(kata_rt::show(tagged), "42");
}

#[test]
fn show_negative_smi() {
    let tagged = tag_int_pub(-7);
    assert_eq!(kata_rt::show(tagged), "-7");
}

#[test]
fn show_bigint() {
    let tagged = tag_int_from_str("99999999999999999999");
    assert_eq!(kata_rt::show(tagged), "99999999999999999999");
}

// ── extern "C" declarations para testes ─────────────────────

unsafe extern "C" {
    fn kata_rt_bi_add(a: i64, b: i64) -> i64;
    fn kata_rt_bi_sub(a: i64, b: i64) -> i64;
    fn kata_rt_bi_mul(a: i64, b: i64) -> i64;
    fn kata_rt_bi_eq(a: i64, b: i64) -> i64;
    fn kata_rt_bi_lt(a: i64, b: i64) -> i64;
    fn kata_rt_bi_gt(a: i64, b: i64) -> i64;
}
