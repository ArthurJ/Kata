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
        let int_val =
            BigInt::parse_bytes(int_part.as_bytes(), 10).unwrap_or_else(|| BigInt::zero());
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
pub fn rat_from_int(int_val: i64) -> BigRational {
    crate::bigint::to_rational(int_val)
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_rat_add(
    a: *const BigRational,
    b: *const BigRational,
) -> *mut BigRational {
    let a = unsafe { &*a };
    let b = unsafe { &*b };
    Box::into_raw(Box::new(a + b))
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_rat_sub(
    a: *const BigRational,
    b: *const BigRational,
) -> *mut BigRational {
    let a = unsafe { &*a };
    let b = unsafe { &*b };
    Box::into_raw(Box::new(a - b))
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_rat_mul(
    a: *const BigRational,
    b: *const BigRational,
) -> *mut BigRational {
    let a = unsafe { &*a };
    let b = unsafe { &*b };
    Box::into_raw(Box::new(a * b))
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_rat_div(
    a: *const BigRational,
    b: *const BigRational,
) -> *mut BigRational {
    let a = unsafe { &*a };
    let b = unsafe { &*b };
    if b.is_zero() {
        panic!("divisão por zero em kata_rt_rat_div");
    }
    Box::into_raw(Box::new(a / b))
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_rat_eq(a: *const BigRational, b: *const BigRational) -> i64 {
    let a = unsafe { &*a };
    let b = unsafe { &*b };
    if a == b { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_rat_lt(a: *const BigRational, b: *const BigRational) -> i64 {
    let a = unsafe { &*a };
    let b = unsafe { &*b };
    if a < b { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_rat_gt(a: *const BigRational, b: *const BigRational) -> i64 {
    let a = unsafe { &*a };
    let b = unsafe { &*b };
    if a > b { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_rat_neq(a: *const BigRational, b: *const BigRational) -> i64 {
    if kata_rt_rat_eq(a, b) == 1 { 0 } else { 1 }
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_rat_le(a: *const BigRational, b: *const BigRational) -> i64 {
    if kata_rt_rat_lt(a, b) == 1 || kata_rt_rat_eq(a, b) == 1 {
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_rat_ge(a: *const BigRational, b: *const BigRational) -> i64 {
    if kata_rt_rat_gt(a, b) == 1 || kata_rt_rat_eq(a, b) == 1 {
        1
    } else {
        0
    }
}

/// show de Rational. Imprime decimal quando denominador é 2^a · 5^b,
/// caso contrário imprime como fração.
pub fn rat_to_string(r: &BigRational) -> String {
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
pub fn rat_to_float(r: &BigRational) -> f64 {
    r.to_f64().unwrap_or(f64::NAN)
}

/// Converte Float para Rational (mais próximo).
pub fn float_to_rat(f: f64) -> BigRational {
    BigRational::from_float(f).unwrap_or_else(|| BigRational::zero())
}
