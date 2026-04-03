#[no_mangle]
pub extern "C" fn kata_rt_add_int(a: i64, b: i64) -> i64 { a.wrapping_add(b) }

#[no_mangle]
pub extern "C" fn kata_rt_sub_int(a: i64, b: i64) -> i64 { a.wrapping_sub(b) }

#[no_mangle]
pub extern "C" fn kata_rt_mul_int(a: i64, b: i64) -> i64 { a.wrapping_mul(b) }

#[no_mangle]
pub extern "C" fn kata_rt_exp_int(a: i64, b: i64) -> i64 {
    if b < 0 { 0 } else { a.wrapping_pow(b as u32) }
}

#[no_mangle]
pub extern "C" fn kata_rt_real_div_int(a: i64, b: i64) -> f64 { a as f64 / b as f64 }

#[no_mangle]
pub extern "C" fn kata_rt_div_int(a: i64, b: i64) -> i64 {
    if b == 0 { 0 } else { a / b }
}

#[no_mangle]
pub extern "C" fn kata_rt_mod_int(a: i64, b: i64) -> i64 {
    if b == 0 { 0 } else { a % b }
}

#[no_mangle]
pub extern "C" fn kata_rt_eq_int(a: i64, b: i64) -> bool { a == b }

#[no_mangle]
pub extern "C" fn kata_rt_gt_int(a: i64, b: i64) -> bool { a > b }

#[no_mangle]
pub extern "C" fn kata_rt_ge_int(a: i64, b: i64) -> bool { a >= b }

#[no_mangle]
pub extern "C" fn kata_rt_lt_int(a: i64, b: i64) -> bool { a < b }

#[no_mangle]
pub extern "C" fn kata_rt_le_int(a: i64, b: i64) -> bool { a <= b }


#[no_mangle]
pub extern "C" fn kata_rt_eq_enum(a: *const u8, b: *const u8) -> bool {
    if a.is_null() || b.is_null() { return false; }
    unsafe { *a == *b }
}

// FLOAT
#[no_mangle]
pub extern "C" fn kata_rt_int_to_float(a: i64) -> f64 { a as f64 }

#[no_mangle]
pub extern "C" fn kata_rt_add_flt(a: f64, b: f64) -> f64 { a + b }

#[no_mangle]
pub extern "C" fn kata_rt_sub_flt(a: f64, b: f64) -> f64 { a - b }

#[no_mangle]
pub extern "C" fn kata_rt_mul_flt(a: f64, b: f64) -> f64 { a * b }

#[no_mangle]
pub extern "C" fn kata_rt_exp_flt(a: f64, b: f64) -> f64 { a.powf(b) }

#[no_mangle]
pub extern "C" fn kata_rt_real_div_flt(a: f64, b: f64) -> f64 { a / b }

#[no_mangle]
pub extern "C" fn kata_rt_div_flt(a: f64, b: f64) -> i64 { (a / b) as i64 }

#[no_mangle]
pub extern "C" fn kata_rt_mod_flt(a: f64, b: f64) -> f64 { a % b }

#[no_mangle]
pub extern "C" fn kata_rt_eq_flt(a: f64, b: f64) -> bool { a == b }

#[no_mangle]
pub extern "C" fn kata_rt_gt_flt(a: f64, b: f64) -> bool { a > b }

#[no_mangle]
pub extern "C" fn kata_rt_ge_flt(a: f64, b: f64) -> bool { a >= b }

#[no_mangle]
pub extern "C" fn kata_rt_lt_flt(a: f64, b: f64) -> bool { a < b }

#[no_mangle]
pub extern "C" fn kata_rt_le_flt(a: f64, b: f64) -> bool { a <= b }

#[no_mangle]
pub extern "C" fn kata_rt_round(a: f64) -> f64 { a.round() }

#[no_mangle]
pub extern "C" fn kata_rt_ceil(a: f64) -> f64 { a.ceil() }

#[no_mangle]
pub extern "C" fn kata_rt_floor(a: f64) -> f64 { a.floor() }
