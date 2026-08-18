//! Operações Float (f64).

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_fadd(a: f64, b: f64) -> f64 {
    a + b
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_fsub(a: f64, b: f64) -> f64 {
    a - b
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_fmul(a: f64, b: f64) -> f64 {
    a * b
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_fdiv(a: f64, b: f64) -> f64 {
    a / b
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_fcmp_eq(a: f64, b: f64) -> i64 {
    if a == b { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_fcmp_neq(a: f64, b: f64) -> i64 {
    if a != b { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_fcmp_lt(a: f64, b: f64) -> i64 {
    if a < b { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_fcmp_le(a: f64, b: f64) -> i64 {
    if a <= b { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_fcmp_gt(a: f64, b: f64) -> i64 {
    if a > b { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_fcmp_ge(a: f64, b: f64) -> i64 {
    if a >= b { 1 } else { 0 }
}

/// Converte Int (SMI-tagged ou BigInt ponteiro) para Float.
/// Recebe o valor cru do codegen (SMI-tagged) e retorna f64.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_int_to_float(val: i64) -> f64 {
    use crate::bigint::{bigint_to_string, decode_smi_pub, is_smi_pub};
    if is_smi_pub(val) {
        decode_smi_pub(val) as f64
    } else {
        bigint_to_string(val).parse::<f64>().unwrap_or(f64::NAN)
    }
}

/// Converte Float para String.
pub fn float_to_string(val: f64) -> String {
    // Remove trailing zeros para output limpo: 5.85 não 5.850000000001
    if val == val.trunc() && val.is_finite() {
        format!("{val:.1}")
    } else {
        format!("{val}")
    }
}

/// `show` de Float — retorna ponteiro C string (ownership transferida).
/// Chamado pelo codegen via `FfiSymbol::FloatToText`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_float_to_text(val: f64) -> *mut std::os::raw::c_char {
    let s = float_to_string(val);
    std::ffi::CString::new(s)
        .unwrap_or_else(|_| std::ffi::CString::new("").expect("CString vazia sempre válida"))
        .into_raw()
}

/// Converte Text (ponteiro C string) para Float (f64).
/// Chamado pelo codegen via `FfiSymbol::TextToFloat`.
/// Suporta notação decimal e exponencial (ex: "3.14", "1e10", "-0.5").
/// Retorna NaN se a string for inválida ou nula.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_text_to_float(s: *const std::os::raw::c_char) -> f64 {
    if s.is_null() {
        return f64::NAN;
    }
    let c_str = unsafe { std::ffi::CStr::from_ptr(s) };
    c_str.to_str().unwrap_or("").parse::<f64>().unwrap_or(f64::NAN)
}

/// Gera um Float aleatório no intervalo [0.0, 1.0).
/// Usado pelo prelude como `rand!()` — action impura que retorna Float.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_rand() -> f64 {
    use rand::Rng;
    rand::rng().random::<f64>()
}

/// Gera um Int aleatório no intervalo [min, max] (inclusivo).
/// Recebe SMI-tagged Ints e retorna SMI-tagged Int.
/// Usado pelo prelude como `rand_int!(min, max)`.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_rand_int(min: i64, max: i64) -> i64 {
    use crate::bigint::{decode_smi_pub, encode_smi_pub, is_smi_pub};
    use rand::Rng;
    let lo = if is_smi_pub(min) { decode_smi_pub(min) } else { min };
    let hi = if is_smi_pub(max) { decode_smi_pub(max) } else { max };
    if lo > hi {
        return encode_smi_pub(lo);
    }
    let n = rand::rng().random_range(lo..=hi);
    encode_smi_pub(n)
}
