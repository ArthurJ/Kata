//! Rational — precisão arbitrária (BigRational).
//!
//! Diferente de Float, toda operação é exata: `1/3 * 3 = 1`, não `0.999...`.
//! Não há rounding, não há erro de runtime.

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};

/// Cria Rational a partir de string bruta do literal (ex: "3.14").
/// Não passa por f64 — preserva precisão do texto.
pub fn rat_from_text(text: &str) -> BigRational {
    // Parser manual: separa parte inteira e decimal
    if let Some((int_part, dec_part)) = text.split_once('.') {
        let int_val = BigInt::parse_bytes(int_part.as_bytes(), 10).unwrap_or_else(BigInt::zero);
        let dec_str = dec_part.trim_end_matches('0');
        if dec_str.is_empty() {
            return BigRational::new(int_val, BigInt::one());
        }
        let dec_val = BigInt::parse_bytes(dec_str.as_bytes(), 10).unwrap_or_else(BigInt::zero);
        let denom = BigInt::from(10).pow(dec_str.len() as u32);
        // Se a parte inteira é negativa, a parte decimal também é
        let signed_dec = if int_val.is_negative() {
            -dec_val
        } else {
            dec_val
        };
        let combined = int_val * &denom + signed_dec;
        BigRational::new(combined, denom)
    } else {
        let n = BigInt::parse_bytes(text.as_bytes(), 10).unwrap_or_else(BigInt::zero);
        BigRational::new(n, BigInt::one())
    }
}

/// Cria Rational a partir de Int (i64 tagged).
#[allow(dead_code)]
pub(crate) fn rat_from_int(int_val: i64) -> BigRational {
    crate::bigint::to_rational(int_val)
}

/// Helper: deref de `*const BigRational` com null-check.
///
/// Hardening do boundary FFI: null (slot não-inicializado que escapou
/// do typeck) é recusado com panic **unwind** e mensagem clara.
///
/// # Safety
/// `ptr` deve ser um ponteiro válido para `BigRational` alocado por
/// `Box::into_raw` ou `kata_rt_rat_*`.
unsafe fn deref_rational<'a>(ptr: *const BigRational) -> &'a BigRational {
    if ptr.is_null() {
        panic!(
            "kata_rt rational: deref de ponteiro null — slot não-inicializado escapou do typeck; isto é um bug do compilador, não do seu código"
        );
    }
    unsafe { &*ptr }
}

/// # Safety
///
/// `a` e `b` devem ser ponteiros válidos para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_add(
    a: *const BigRational,
    b: *const BigRational,
) -> *mut BigRational {
    // SAFETY: caller (JIT codegen) garante ponteiros válidos.
    let a = unsafe { deref_rational(a) };
    let b = unsafe { deref_rational(b) };
    Box::into_raw(Box::new(a + b))
}

/// # Safety
///
/// `a` e `b` devem ser ponteiros válidos para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_sub(
    a: *const BigRational,
    b: *const BigRational,
) -> *mut BigRational {
    // SAFETY: caller (JIT codegen) garante ponteiros válidos.
    let a = unsafe { deref_rational(a) };
    let b = unsafe { deref_rational(b) };
    Box::into_raw(Box::new(a - b))
}

/// # Safety
///
/// `a` e `b` devem ser ponteiros válidos para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_mul(
    a: *const BigRational,
    b: *const BigRational,
) -> *mut BigRational {
    // SAFETY: caller (JIT codegen) garante ponteiros válidos.
    let a = unsafe { deref_rational(a) };
    let b = unsafe { deref_rational(b) };
    Box::into_raw(Box::new(a * b))
}

/// # Safety
///
/// `a` e `b` devem ser ponteiros válidos para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_div(
    a: *const BigRational,
    b: *const BigRational,
) -> *mut BigRational {
    // SAFETY: caller (JIT codegen) garante ponteiros válidos.
    let a = unsafe { deref_rational(a) };
    let b = unsafe { deref_rational(b) };
    if b.is_zero() {
        panic!("divisão por zero em kata_rt_rat_div");
    }
    Box::into_raw(Box::new(a / b))
}

/// # Safety
///
/// `a` e `b` devem ser ponteiros válidos para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_eq(a: *const BigRational, b: *const BigRational) -> i64 {
    // SAFETY: caller (JIT codegen) garante ponteiros válidos.
    let a = unsafe { deref_rational(a) };
    let b = unsafe { deref_rational(b) };
    if a == b { 1 } else { 0 }
}

/// # Safety
///
/// `a` e `b` devem ser ponteiros válidos para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_lt(a: *const BigRational, b: *const BigRational) -> i64 {
    // SAFETY: caller (JIT codegen) garante ponteiros válidos.
    let a = unsafe { deref_rational(a) };
    let b = unsafe { deref_rational(b) };
    if a < b { 1 } else { 0 }
}

/// # Safety
///
/// `a` e `b` devem ser ponteiros válidos para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_gt(a: *const BigRational, b: *const BigRational) -> i64 {
    // SAFETY: caller (JIT codegen) garante ponteiros válidos.
    let a = unsafe { deref_rational(a) };
    let b = unsafe { deref_rational(b) };
    if a > b { 1 } else { 0 }
}

/// # Safety
///
/// `a` e `b` devem ser ponteiros válidos para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_neq(a: *const BigRational, b: *const BigRational) -> i64 {
    // SAFETY: caller (JIT codegen) garante ponteiros válidos.
    unsafe { if kata_rt_rat_eq(a, b) == 1 { 0 } else { 1 } }
}

/// # Safety
///
/// `a` e `b` devem ser ponteiros válidos para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_le(a: *const BigRational, b: *const BigRational) -> i64 {
    // SAFETY: caller (JIT codegen) garante ponteiros válidos.
    unsafe {
        if kata_rt_rat_lt(a, b) == 1 || kata_rt_rat_eq(a, b) == 1 {
            1
        } else {
            0
        }
    }
}

/// # Safety
///
/// `a` e `b` devem ser ponteiros válidos para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_ge(a: *const BigRational, b: *const BigRational) -> i64 {
    // SAFETY: caller (JIT codegen) garante ponteiros válidos.
    unsafe {
        if kata_rt_rat_gt(a, b) == 1 || kata_rt_rat_eq(a, b) == 1 {
            1
        } else {
            0
        }
    }
}

/// show de Rational. Imprime decimal quando denominador é 2^a · 5^b,
/// caso contrário imprime como fração.
pub(crate) fn rat_to_string(r: &BigRational) -> String {
    // Se denominador é 1, é inteiro
    if r.denom() == &BigInt::one() {
        return r.numer().to_string();
    }
    // Verifica se denominador é da forma 2^a * 5^b
    if let Some(decimal) = try_decimal(r) {
        return decimal;
    }
    // Caso contrário, fração
    format!("{}/{}", r.numer(), r.denom())
}

/// Tenta representar como decimal exato se denominador é 2^a * 5^b.
fn try_decimal(r: &BigRational) -> Option<String> {
    let denom = r.denom();
    let mut d = denom.clone();
    let two = BigInt::from(2);
    let five = BigInt::from(5);

    let mut exp2: u32 = 0;
    let mut exp5: u32 = 0;

    while &d % &two == BigInt::zero() {
        d /= &two;
        exp2 += 1;
    }
    while &d % &five == BigInt::zero() {
        d /= &five;
        exp5 += 1;
    }

    if d != BigInt::one() {
        return None;
    }

    // É decimal exato. Calcula representação decimal.
    let max_exp = exp2.max(exp5);
    let scale = BigInt::from(10).pow(max_exp);
    let scaled = r.numer() * &scale / denom;

    // Formata com casas decimais = max_exp
    let s = scaled.to_string();
    let (neg, digits) = if let Some(stripped) = s.strip_prefix('-') {
        (true, stripped)
    } else {
        (false, s.as_str())
    };

    if max_exp == 0 {
        return Some(s);
    }

    let digits_len = digits.len() as u32;
    let int_part: String;
    let dec_part: String;

    if digits_len > max_exp {
        let split = digits.len() - max_exp as usize;
        int_part = digits[..split].to_string();
        dec_part = digits[split..].to_string();
    } else {
        int_part = "0".to_string();
        dec_part = format!(
            "{digits:0>max_exp_usize$}",
            max_exp_usize = max_exp as usize
        );
    }

    // Remove trailing zeros da parte decimal
    let dec_trimmed = dec_part.trim_end_matches('0');
    if dec_trimmed.is_empty() {
        Some(format!("{}{}", if neg { "-" } else { "" }, int_part))
    } else {
        Some(format!(
            "{}{}.{}",
            if neg { "-" } else { "" },
            int_part,
            dec_trimmed
        ))
    }
}

/// Converte Rational para Float.
pub(crate) fn rat_to_float(r: &BigRational) -> f64 {
    r.to_f64().unwrap_or(f64::NAN)
}

/// Converte Float para Rational (mais próximo).
pub(crate) fn float_to_rat(f: f64) -> BigRational {
    BigRational::from_float(f).unwrap_or_else(BigRational::zero)
}

// ── Wrappers C-ABI para codegen ───────────────────────────

/// `show` de Rational — retorna ponteiro C string.
/// Chamado pelo codegen via `FfiSymbol::RatShow`.
///
/// # Safety
///
/// `r` deve ser ponteiro válido para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_show(r: *const BigRational) -> *mut std::os::raw::c_char {
    // SAFETY: caller (JIT codegen) garante ponteiro válido.
    let r = unsafe { deref_rational(r) };
    let s = rat_to_string(r);
    std::ffi::CString::new(s)
        .unwrap_or_else(|_| std::ffi::CString::new("").expect("CString vazia sempre válida"))
        .into_raw()
}

/// `show` de Rational a partir de ponteiro bruto (i64).
/// Versão para o driver que não tem acesso ao tipo `BigRational`.
///
/// # Safety
///
/// `r_raw` deve ser um i64 que representa um ponteiro válido para `BigRational`.
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn kata_rt_rat_show_raw(r_raw: i64) -> *mut std::os::raw::c_char {
    let r = r_raw as *const BigRational;
    // SAFETY: caller (driver) garante que r_raw é ponteiro válido.
    unsafe { kata_rt_rat_show(r) }
}

/// Converte Rational para Float.
/// Chamado pelo codegen via `FfiSymbol::RatToFloat`.
///
/// # Safety
///
/// `r` deve ser ponteiro válido para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_to_float(r: *const BigRational) -> f64 {
    // SAFETY: caller (JIT codegen) garante ponteiro válido.
    let r = unsafe { deref_rational(r) };
    rat_to_float(r)
}

/// Converte Float para Rational (retorna ponteiro).
/// Chamado pelo codegen via `FfiSymbol::RatFromFloat`.
///
/// # Safety
///
/// Esta função é safe para qualquer `f64` — não dereferencia ponteiros.
/// Marcada `unsafe` apenas por convenção C-ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_from_float(f: f64) -> *mut BigRational {
    let r = float_to_rat(f);
    Box::into_raw(Box::new(r))
}

/// Cria Rational a partir de string bruta do literal (ponteiro C + len).
/// Chamado pelo codegen via `FfiSymbol::RatLiteral`.
/// Não passa por f64 — preserva precisão do texto.
///
/// # Safety
///
/// `s` deve ser um ponteiro C string válido (nulo-terminado) ou NULL.
/// Se não for NULL, `len` deve ser o comprimento em bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_literal(
    s: *const std::os::raw::c_char,
    len: i64,
) -> *mut BigRational {
    let bytes = if s.is_null() || len <= 0 {
        &b""[..]
    } else {
        // SAFETY: caller (JIT codegen) garante ponteiro e len válidos.
        unsafe { std::slice::from_raw_parts(s as *const u8, len as usize) }
    };
    let text = std::str::from_utf8(bytes).unwrap_or("");
    let r = rat_from_text(text);
    Box::into_raw(Box::new(r))
}

/// Retorna o zero do tipo Rational. Identidade aditiva de NUM.
/// Recebe `self` por convenção da assinatura `zero :: Self => Self`, mas o ignora.
///
/// # Safety
///
/// Marcada `unsafe` apenas por convenção C-ABI. Não dereferencia ponteiros.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rat_zero(_val: *const BigRational) -> *mut BigRational {
    Box::into_raw(Box::new(BigRational::zero()))
}

/// Converte Int tagged para Rational (retorna ponteiro).
/// Chamado pelo codegen via `FfiSymbol::IntToRational`.
///
/// # Safety
///
/// Esta função é safe para qualquer `i64` — não dereferencia ponteiros.
/// Marcada `unsafe` apenas por convenção C-ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_int_to_rational(val: i64) -> *mut BigRational {
    let r = crate::bigint::to_rational(val);
    Box::into_raw(Box::new(r))
}

/// Converte Rational para Int (SMI-tagged). Trunca em direção a zero.
/// Recebe ponteiro para BigRational. Retorna Int tagged (SMI ou BigInt ptr).
///
/// # Safety
///
/// `r` deve ser ponteiro válido para `BigRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kata_rt_rational_to_int(r: *const BigRational) -> i64 {
    use crate::bigint::{alloc_bigint, encode_smi_pub, fits_smi_pub};
    use num_bigint::ToBigInt;

    // SAFETY: caller (codegen) garante ponteiro válido.
    let r = unsafe { deref_rational(r) };

    // Trunca em direção a zero: numerador / denominador (divisão inteira).
    let trunc = r.numer() / r.denom();
    let big = trunc.to_bigint().unwrap_or_else(BigInt::zero);

    if let Some(small) = big.to_i64()
        && fits_smi_pub(small)
    {
        return encode_smi_pub(small);
    }
    alloc_bigint(big)
}
