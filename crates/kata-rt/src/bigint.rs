//! BigInt com SMI tagging — representação nativa de Int.
//!
//! **Invariante de transparência:** o compilador vê `i64` em todo o pipeline.
//! O runtime decide representação: SMI inline (zero alocação) ou heap BigInt.
//!
//! ## SMI Tagging
//!
//! O bit menos significativo (LSB) do `i64` é a tag:
//! - **LSB = 1** → SMI (Small Integer). O valor está embutido: `value >> 1`.
//!   O range de SMI é `[-2^62, 2^62 - 1]` (63 bits de payload com sinal).
//! - **LSB = 0** → Ponteiro para heap BigInt. O `i64` é o endereço do bloco
//!   heap (alinhado a 8 bytes, então LSB é sempre 0).
//!
//! O codegen nunca precisa distinguir — todas as operações (`bi_add`,
//! `bi_eq`, etc.) fazem dispatch interno: se ambos operandos são SMI e o
//! resultado cabe em SMI, opera inline; caso contrário, promove para heap.
//!
//! ## Por que SMI tagging
//!
//! - Valores pequenos (esmagadora maioria) são zero-allocation.
//! - O compilador não precisa saber — vê `i64`, codegen passa `i64`.
//! - BigInt de precisão arbitrária para valores grandes, sem overflow.

use num_bigint::BigInt;
use num_traits::{One, ToPrimitive, Zero};

// ── Constantes de tagging ─────────────────────────────────

/// Tag SMI: LSB = 1.
const SMI_TAG: u64 = 1;
/// Máscara para extrair payload de SMI: `value >> 1`.
/// Usamos shift right 1 para obter o payload, shift left 1 + tag para codificar.
const SMI_SHIFT: u32 = 1;

/// Limite superior de SMI: 2^62 - 1 (maior i62 não-negativo).
const SMI_MAX: i64 = (1i64 << 62) - 1;
/// Limite inferior de SMI: -2^62 (menor i62).
const SMI_MIN: i64 = -(1i64 << 62);

/// Verifica se um i64 é SMI (LSB = 1).
#[inline]
fn is_smi(val: i64) -> bool {
    (val as u64) & SMI_TAG == SMI_TAG
}

/// Codifica um i64 como SMI. Não verifica range — caller deve checar.
#[inline]
fn encode_smi(val: i64) -> i64 {
    (val << SMI_SHIFT) | (SMI_TAG as i64)
}

/// Decodifica SMI para i64 (payload).
#[inline]
fn decode_smi(val: i64) -> i64 {
    val >> SMI_SHIFT
}

/// Verifica se um i64 cabe em SMI.
#[inline]
fn fits_smi(val: i64) -> bool {
    (SMI_MIN..=SMI_MAX).contains(&val)
}

// ── Heap BigInt ───────────────────────────────────────────

// Layout do bloco heap para BigInt.
// Usamos um header com refcount (para futura integração com ARC) seguido
// dos digits do BigInt. O refcount é mantido em 1 (sem sharing).
//
// Usamos `Box<BigInt>` diretamente — o `i64` retornado é um
// ponteiro não-null para o heap. O LSB é 0 (alinhamento natural de Box).
//
// **Invariante:** `i64` com LSB=0 é sempre ponteiro válido para `Box<BigInt>`.
// O caller nunca deve aritmética de ponteiros — usa as funções deste módulo.

// SAFETY: FFI — o codegen chama estas funções via `#[unsafe(no_mangle)]`.
/// Aloca BigInt no heap e retorna o ponteiro como i64.
/// O LSB será 0 (alinhamento de Box<BigInt> é ≥ 8 bytes).
pub(crate) fn alloc_bigint(n: BigInt) -> i64 {
    let boxed = Box::new(n);
    let ptr = Box::into_raw(boxed) as i64;
    // LSB deve ser 0 — Box<BigInt> tem alinhamento ≥ 8.
    debug_assert!(!is_smi(ptr), "BigInt pointer colide com SMI tag");
    ptr
}

/// Recupera &BigInt de um ponteiro heap (LSB = 0).
///
/// # Safety
/// `val` deve ser um ponteiro válido de `alloc_bigint` ainda não liberado.
unsafe fn deref_bigint<'a>(val: i64) -> &'a BigInt {
    unsafe { &*(val as *const BigInt) }
}

// ── API pública C-ABI ─────────────────────────────────────

/// Cria um Int a partir de string decimal. Usado para literais.
/// Se cabe em SMI, retorna SMI; caso contrário, aloca heap BigInt.
///
/// # Safety
/// `s` deve ser um ponteiro válido para string C null-terminated.
/// (Chamado internamente — não expomos C string ainda.)
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tag_int(val: i64) -> i64 {
    if fits_smi(val) {
        encode_smi(val)
    } else {
        alloc_bigint(BigInt::from(val))
    }
}

/// Cria um Int a partir de texto bruto (ponteiro C + len).
/// Versão C-ABI para o codegen — suporta decimal, hex, octal, bin, underscore.
/// Chamado pelo codegen para literais que não cabem em SMI (BigInts).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tag_int_from_str(s: *const std::os::raw::c_char, len: i64) -> i64 {
    let bytes = if s.is_null() || len <= 0 {
        &b"0"[..]
    } else {
        unsafe { std::slice::from_raw_parts(s as *const u8, len as usize) }
    };
    let text = std::str::from_utf8(bytes).unwrap_or("0");
    tag_int_from_str(text)
}

/// Tenta parsear texto como BigInt.
/// Suporta decimal, hex (0x), octal (0o), bin (0b), separador _.
/// Retorna None se o texto for inválido.
fn parse_int_to_bigint(text: &str) -> Option<BigInt> {
    let cleaned = text.replace('_', "");
    // Extrai sinal antes do dispatch de base
    let (sign, digits) = if let Some(rest) = cleaned.strip_prefix('-') {
        (-1i32, rest)
    } else if let Some(rest) = cleaned.strip_prefix('+') {
        (1i32, rest)
    } else {
        (1i32, cleaned.as_str())
    };
    let n = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        BigInt::parse_bytes(hex.as_bytes(), 16)
    } else if let Some(oct) = digits
        .strip_prefix("0o")
        .or_else(|| digits.strip_prefix("0O"))
    {
        BigInt::parse_bytes(oct.as_bytes(), 8)
    } else if let Some(bin) = digits
        .strip_prefix("0b")
        .or_else(|| digits.strip_prefix("0B"))
    {
        BigInt::parse_bytes(bin.as_bytes(), 2)
    } else if let Some(dec) = digits
        .strip_prefix("0d")
        .or_else(|| digits.strip_prefix("0D"))
    {
        BigInt::parse_bytes(dec.as_bytes(), 10)
    } else {
        BigInt::parse_bytes(digits.as_bytes(), 10)
    }?;
    let n = if sign < 0 { -n } else { n };
    Some(n)
}

/// Codifica um BigInt como Int tagged (SMI ou heap pointer).
fn bigint_to_tagged(n: BigInt) -> i64 {
    if let Some(small) = n.to_i64()
        && fits_smi(small)
    {
        return encode_smi(small);
    }
    alloc_bigint(n)
}

/// Cria um Int a partir de texto bruto do literal.
/// Suporta decimal, hex (0x), octal (0o), bin (0b), separador _.
/// (Versão interna — chamada pelo codegen ao lowerar IntLit.)
/// Panica se o texto for inválido — assumido válido em compile-time.
pub fn tag_int_from_str(text: &str) -> i64 {
    let n = parse_int_to_bigint(text).expect("número inválido");
    bigint_to_tagged(n)
}

/// Soma dois Int. Se ambos SMI e resultado cabe, opera inline.
/// Caso contrário, promove para BigInt.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_add(a: i64, b: i64) -> i64 {
    if is_smi(a) && is_smi(b) {
        let ra = decode_smi(a);
        let rb = decode_smi(b);
        match ra.checked_add(rb) {
            Some(result) if fits_smi(result) => return encode_smi(result),
            _ => {}
        }
        // Overflow SMI — promove
        let result = BigInt::from(ra) + BigInt::from(rb);
        return alloc_bigint(result);
    }
    // Pelo menos um é BigInt
    let result = unsafe {
        let ba = if is_smi(a) {
            BigInt::from(decode_smi(a))
        } else {
            deref_bigint(a).clone()
        };
        let bb = if is_smi(b) {
            BigInt::from(decode_smi(b))
        } else {
            deref_bigint(b).clone()
        };
        ba + bb
    };
    // Se resultado cabe em SMI, retorna SMI (evita heap desnecessário)
    if let Some(small) = result.to_i64()
        && fits_smi(small)
    {
        return encode_smi(small);
    }
    alloc_bigint(result)
}

/// Subtração dois Int.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_sub(a: i64, b: i64) -> i64 {
    if is_smi(a) && is_smi(b) {
        let ra = decode_smi(a);
        let rb = decode_smi(b);
        match ra.checked_sub(rb) {
            Some(result) if fits_smi(result) => return encode_smi(result),
            _ => {}
        }
        let result = BigInt::from(ra) - BigInt::from(rb);
        return alloc_bigint(result);
    }
    let result = unsafe {
        let ba = if is_smi(a) {
            BigInt::from(decode_smi(a))
        } else {
            deref_bigint(a).clone()
        };
        let bb = if is_smi(b) {
            BigInt::from(decode_smi(b))
        } else {
            deref_bigint(b).clone()
        };
        ba - bb
    };
    if let Some(small) = result.to_i64()
        && fits_smi(small)
    {
        return encode_smi(small);
    }
    alloc_bigint(result)
}

/// Multiplicação dois Int.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_mul(a: i64, b: i64) -> i64 {
    if is_smi(a) && is_smi(b) {
        let ra = decode_smi(a);
        let rb = decode_smi(b);
        match ra.checked_mul(rb) {
            Some(result) if fits_smi(result) => return encode_smi(result),
            _ => {}
        }
        let result = BigInt::from(ra) * BigInt::from(rb);
        return alloc_bigint(result);
    }
    let result = unsafe {
        let ba = if is_smi(a) {
            BigInt::from(decode_smi(a))
        } else {
            deref_bigint(a).clone()
        };
        let bb = if is_smi(b) {
            BigInt::from(decode_smi(b))
        } else {
            deref_bigint(b).clone()
        };
        ba * bb
    };
    if let Some(small) = result.to_i64()
        && fits_smi(small)
    {
        return encode_smi(small);
    }
    alloc_bigint(result)
}

/// Divisão inteira. Pânico se divisor é zero (o typeck deve prevenir
/// via NonZero refined). Divisão por zero = abort.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_div(a: i64, b: i64) -> i64 {
    let result = unsafe {
        let ba = if is_smi(a) {
            BigInt::from(decode_smi(a))
        } else {
            deref_bigint(a).clone()
        };
        let bb = if is_smi(b) {
            BigInt::from(decode_smi(b))
        } else {
            deref_bigint(b).clone()
        };
        if bb.is_zero() {
            panic!("divisão por zero em kata_rt_bi_div");
        }
        ba / bb
    };
    if let Some(small) = result.to_i64()
        && fits_smi(small)
    {
        return encode_smi(small);
    }
    alloc_bigint(result)
}

/// Igualdade. Retorna 1 (True) ou 0 (False) como i64 (Boolean::True/False).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_eq(a: i64, b: i64) -> i64 {
    if is_smi(a) && is_smi(b) {
        return if decode_smi(a) == decode_smi(b) { 1 } else { 0 };
    }
    let result = unsafe {
        let ba: BigInt = if is_smi(a) {
            BigInt::from(decode_smi(a))
        } else {
            deref_bigint(a).clone()
        };
        let bb: BigInt = if is_smi(b) {
            BigInt::from(decode_smi(b))
        } else {
            deref_bigint(b).clone()
        };
        ba == bb
    };
    if result { 1 } else { 0 }
}

/// Menor que. Retorna 1 (True) ou 0 (False).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_lt(a: i64, b: i64) -> i64 {
    if is_smi(a) && is_smi(b) {
        return if decode_smi(a) < decode_smi(b) { 1 } else { 0 };
    }
    let result = unsafe {
        let ba: BigInt = if is_smi(a) {
            BigInt::from(decode_smi(a))
        } else {
            deref_bigint(a).clone()
        };
        let bb: BigInt = if is_smi(b) {
            BigInt::from(decode_smi(b))
        } else {
            deref_bigint(b).clone()
        };
        ba < bb
    };
    if result { 1 } else { 0 }
}

/// Maior que. Retorna 1 (True) ou 0 (False).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_gt(a: i64, b: i64) -> i64 {
    // a > b == b < a == !(a <= b)
    if is_smi(a) && is_smi(b) {
        return if decode_smi(a) > decode_smi(b) { 1 } else { 0 };
    }
    let result = unsafe {
        let ba: BigInt = if is_smi(a) {
            BigInt::from(decode_smi(a))
        } else {
            deref_bigint(a).clone()
        };
        let bb: BigInt = if is_smi(b) {
            BigInt::from(decode_smi(b))
        } else {
            deref_bigint(b).clone()
        };
        ba > bb
    };
    if result { 1 } else { 0 }
}

/// Desigualdade. Retorna 1 (True) ou 0 (False).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_neq(a: i64, b: i64) -> i64 {
    if kata_rt_bi_eq(a, b) == 1 { 0 } else { 1 }
}

/// Menor ou igual. Retorna 1 (True) ou 0 (False).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_le(a: i64, b: i64) -> i64 {
    if kata_rt_bi_lt(a, b) == 1 || kata_rt_bi_eq(a, b) == 1 {
        1
    } else {
        0
    }
}

/// Maior ou igual. Retorna 1 (True) ou 0 (False).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_ge(a: i64, b: i64) -> i64 {
    if kata_rt_bi_gt(a, b) == 1 || kata_rt_bi_eq(a, b) == 1 {
        1
    } else {
        0
    }
}

/// Retorna o zero do tipo Int (SMI-tagged). Identidade aditiva de NUM.
/// Recebe `self` por convenção da assinatura `zero :: Self => Self`, mas o ignora.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_zero(_val: i64) -> i64 {
    encode_smi(0)
}

/// Converte Int para String (para show/println).
/// Retorna String alocada no heap (propriedade transferida ao caller).
pub fn bigint_to_string(val: i64) -> String {
    if is_smi(val) {
        decode_smi(val).to_string()
    } else {
        unsafe { deref_bigint(val).to_string() }
    }
}

/// `show` de Int — retorna string formatada.
/// (Interno — o codegen chama `kata_rt_int_to_text` que produz ponteiro C.)
pub fn show(val: i64) -> String {
    bigint_to_string(val)
}

/// Converte Int para Rational (para interoperabilidade).
pub(crate) fn to_rational(val: i64) -> num_rational::BigRational {
    let n = if is_smi(val) {
        BigInt::from(decode_smi(val))
    } else {
        unsafe { deref_bigint(val).clone() }
    };
    num_rational::BigRational::new(n, BigInt::one())
}

// ── Wrappers C-ABI para codegen ───────────────────────────

/// `show` de Int — retorna ponteiro C string (ownership transferida).
/// Chamado pelo codegen via `FfiSymbol::BiShow`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_show(val: i64) -> *mut std::os::raw::c_char {
    let s = bigint_to_string(val);
    std::ffi::CString::new(s)
        .unwrap_or_else(|_| std::ffi::CString::new("").expect("CString vazia sempre válida"))
        .into_raw()
}

/// Converte Int tagged para Rational (ponteiro).
/// Chamado pelo codegen via `FfiSymbol::BiToRational`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_bi_to_rational(val: i64) -> *mut num_rational::BigRational {
    let r = to_rational(val);
    Box::into_raw(Box::new(r))
}

/// Converte Int tagged para Text (ponteiro C string).
/// Chamado pelo codegen via `FfiSymbol::IntToText`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_int_to_text(val: i64) -> *mut std::os::raw::c_char {
    let s = bigint_to_string(val);
    std::ffi::CString::new(s)
        .unwrap_or_else(|_| std::ffi::CString::new("").expect("CString vazia sempre válida"))
        .into_raw()
}

/// Converte Text (ponteiro C string) para Int tagged.
/// Chamado pelo codegen via `FfiSymbol::TextToInt`.
/// Suporta decimal, hex (0x), octal (0o), bin (0b), separador `_`.
/// Retorna 0 se a string for inválida ou nula.
///
/// # Safety
///
/// `s` deve ser um ponteiro válido para uma string C terminada em NUL,
/// ou um ponteiro nulo (que retorna 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_text_to_int(s: *const std::os::raw::c_char) -> i64 {
    if s.is_null() {
        return encode_smi(0);
    }
    let c_str = unsafe { std::ffi::CStr::from_ptr(s) };
    let text = c_str.to_str().unwrap_or("0");
    tag_int_from_str(text)
}

// ── Debug helpers (não C-ABI) ─────────────────────────────

/// Verifica se valor é SMI. Para testes e debug.
pub fn is_smi_pub(val: i64) -> bool {
    is_smi(val)
}

/// Decodifica SMI para i64. Para testes e debug.
pub fn decode_smi_pub(val: i64) -> i64 {
    decode_smi(val)
}

/// Codifica i64 como SMI. Para testes e debug.
pub fn encode_smi_pub(val: i64) -> i64 {
    encode_smi(val)
}

/// Verifica se i64 cabe em SMI. Para testes e debug.
pub fn fits_smi_pub(val: i64) -> bool {
    fits_smi(val)
}

/// Cria Int a partir de i64. Para testes e debug.
pub fn tag_int_pub(val: i64) -> i64 {
    if fits_smi(val) {
        encode_smi(val)
    } else {
        alloc_bigint(BigInt::from(val))
    }
}

/// `kata_rt_try_int(s: *const c_char) -> i64` — converte Text para Int sem panicar.
///
/// Retorna Result box: Ok(Int tagged) ou Err(Text "número inválido").
/// Ao contrário de `kata_rt_text_to_int`, não panica em input inválido.
///
/// # Safety
/// `s` deve ser um ponteiro válido para C string nul-terminada, ou NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_try_int(s: *const std::os::raw::c_char) -> i64 {
    use crate::file::{alloc_result_box, alloc_text};

    if s.is_null() {
        return alloc_result_box(1, alloc_text("número inválido"));
    }
    let c_str = unsafe { std::ffi::CStr::from_ptr(s) };
    let text = match c_str.to_str() {
        Ok(t) => t,
        Err(_) => return alloc_result_box(1, alloc_text("número inválido")),
    };
    match parse_int_to_bigint(text) {
        Some(n) => alloc_result_box(0, bigint_to_tagged(n)),
        None => alloc_result_box(1, alloc_text("número inválido")),
    }
}
