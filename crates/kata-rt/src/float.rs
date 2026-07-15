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
