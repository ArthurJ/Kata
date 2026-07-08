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
