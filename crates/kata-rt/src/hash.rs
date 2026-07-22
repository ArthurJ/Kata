//! Hash — FNV-1a 64-bit para Int, Text, e Rational.
//!
//! Usado para hashing de chaves em estruturas como Map e Set.
//! O hash é determinístico e content-based (não depende do endereço do ponteiro).
//!
//! ## SMI Tagging
//!
//! Valores `i64` do codegen são SMI-tagged:
//! - LSB = 1 → SMI: `value = (val - 1) >> 1` (ou `val >> 1`, pois bit 0 é descartado).
//! - LSB = 0 → Ponteiro para BigInt no heap.
//!
//! Para SMIs, hasheamos os bits crus do valor untagged.
//! Para BigInts, hasheamos o ponteiro (placeholder até BigInt ter byte representation).

// ── Constantes FNV-1a 64-bit ───────────────────────────────

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// Verifica se um i64 é SMI (LSB = 1).
#[inline]
fn is_smi(val: i64) -> bool {
    (val as u64) & 1 == 1
}

/// Decodifica SMI para i64: `(val - 1) >> 1`.
#[inline]
fn decode_smi(val: i64) -> i64 {
    (val - 1) >> 1
}

/// FNV-1a 64-bit sobre bytes arbitrários.
fn fnv1a_bytes(bytes: &[u8]) -> u64 {
    let mut hash: u64 = FNV_OFFSET;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

// ── API pública C-ABI ─────────────────────────────────────

/// Hash de Int (i64 SMI-tagged).
///
/// Se SMI: untag e hasheia os bits crus do valor.
/// Se BigInt: hasheia o valor do ponteiro (placeholder).
///
/// # Safety
///
/// Esta função é safe para qualquer `i64` — não dereferencia ponteiros.
/// Marcada `unsafe` apenas por convenção C-ABI.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_hash_int(val: i64) -> i64 {
    if is_smi(val) {
        let raw = decode_smi(val);
        fnv1a_bytes(&raw.to_le_bytes()) as i64
    } else {
        // BigInt pointer — hash the pointer value (placeholder).
        fnv1a_bytes(&val.to_le_bytes()) as i64
    }
}

/// Hash de Text (ponteiro C string, null-terminated).
///
/// Hasheia os bytes do conteúdo — **content-based, não pointer-based**.
/// Ponteiro nulo retorna FNV_OFFSET.
///
/// # Safety
///
/// `str_ptr` deve ser um ponteiro C string válido (nulo-terminado) ou 0 (NULL).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_hash_text(str_ptr: i64) -> i64 {
    if str_ptr == 0 {
        return FNV_OFFSET as i64;
    }
    // SAFETY: caller (JIT codegen) garante que str_ptr é um ponteiro C string válido.
    let bytes = unsafe { std::ffi::CStr::from_ptr(str_ptr as *const std::os::raw::c_char) }
        .to_bytes();
    fnv1a_bytes(bytes) as i64
}

/// Hash de Rational (ponteiro para struct com numer no offset 0, denom no offset 8).
///
/// Hasheia numer e denom separadamente e XOR os resultados.
/// Ponteiro nulo retorna FNV_OFFSET.
///
/// # Safety
///
/// `rat_ptr` deve ser um ponteiro válido para uma struct com layout:
/// - offset 0: numer (i64)
/// - offset 8: denom (i64)
/// Ou 0 (NULL).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_hash_rational(rat_ptr: i64) -> i64 {
    if rat_ptr == 0 {
        return FNV_OFFSET as i64;
    }
    // SAFETY: caller (JIT codegen) garante que rat_ptr é um ponteiro válido.
    let numer = unsafe {
        std::ptr::read_unaligned(rat_ptr as *const i64)
    };
    let denom = unsafe {
        std::ptr::read_unaligned((rat_ptr as *const u8).add(8) as *const i64)
    };
    let h_numer = fnv1a_bytes(&numer.to_le_bytes());
    let h_denom = fnv1a_bytes(&denom.to_le_bytes());
    (h_numer ^ h_denom) as i64
}