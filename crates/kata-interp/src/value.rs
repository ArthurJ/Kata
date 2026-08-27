//! Representação de valores no interpretador — `i64` cru, mesmo formato do runtime.
//!
//! SMI-tagging é preservado: Int pequeno é `(val << 1) | 1`, Float é
//! `f64::to_bits() as i64`, ponteiros (List, Struct, Text, Array) são
//! `i64` ponteiros para a arena. Isso permite chamar as mesmas funções
//! C-ABI de `kata-rt` diretamente.

/// Valor no interpretador — i64 cru, mesmo formato do runtime.
pub(crate) type Value = i64;

// ── SMI helpers ──────────────────────────────────────────────

/// Codifica um i64 como SMI: shift left 1, OR com 1.
#[inline]
pub(crate) fn encode_smi(val: i64) -> i64 {
    (val << 1) | 1
}

/// Decodifica um SMI para i64.
#[inline]
pub(crate) fn decode_smi(val: i64) -> i64 {
    val >> 1
}

/// Verifica se um i64 é SMI (LSB = 1).
#[inline]
pub(crate) fn is_smi(val: i64) -> bool {
    (val as u64) & 1 == 1
}

/// Verifica se um valor i64 cabe em SMI (62 bits + 1 bit tag).
#[inline]
pub(crate) fn fits_smi(val: i64) -> bool {
    (-(1i64 << 62)..(1i64 << 62)).contains(&val)
}

// ── Float helpers ────────────────────────────────────────────

/// Converte f64 para Value (reinterpretar bits).
#[inline]
pub(crate) fn f64_to_value(f: f64) -> Value {
    f.to_bits() as i64
}

/// Converte Value para f64 (reinterpretar bits).
#[inline]
pub(crate) fn value_to_f64(v: Value) -> f64 {
    f64::from_bits(v as u64)
}
