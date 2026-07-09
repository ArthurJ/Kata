//! Helpers de SMI tagging para compile-time.
//!
//! Duplicados do runtime (`kata-rt/src/bigint.rs`) para uso em compile-time
//! quando o codegen precisa decidir se um literal cabe em SMI ou precisa
//! de BigInt.

/// Verifica se um valor i64 cabe em SMI (62 bits + 1 bit tag).
pub(crate) fn fits_smi(val: i64) -> bool {
    (-(1i64 << 62)..(1i64 << 62)).contains(&val)
}

/// Codifica um valor i64 como SMI: shift left 1, OR com 1.
pub(crate) fn encode_smi(val: i64) -> i64 {
    (val << 1) | 1
}

/// Parseia um literal inteiro (decimal/hex/oct/bin, com underscore).
/// Retorna None se o número não cabe em i64 (BigInt).
pub(crate) fn parse_int_literal(text: &str) -> Option<i64> {
    let cleaned = text.replace('_', "");
    let (sign, digits) = if let Some(rest) = cleaned.strip_prefix('-') {
        (-1i64, rest)
    } else if let Some(rest) = cleaned.strip_prefix('+') {
        (1i64, rest)
    } else {
        (1i64, cleaned.as_str())
    };

    let n = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        i64::from_str_radix(hex, 16).ok()
    } else if let Some(oct) = digits
        .strip_prefix("0o")
        .or_else(|| digits.strip_prefix("0O"))
    {
        i64::from_str_radix(oct, 8).ok()
    } else if let Some(bin) = digits
        .strip_prefix("0b")
        .or_else(|| digits.strip_prefix("0B"))
    {
        i64::from_str_radix(bin, 2).ok()
    } else if let Some(dec) = digits
        .strip_prefix("0d")
        .or_else(|| digits.strip_prefix("0D"))
    {
        dec.parse::<i64>().ok()
    } else {
        digits.parse::<i64>().ok()
    };

    // Retorna Some(val) se parseou como i64, None se é BigInt grande.
    n.map(|v| v * sign)
}
