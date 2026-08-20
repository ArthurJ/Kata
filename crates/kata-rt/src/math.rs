//! Funções matemáticas para o módulo `math.kata`.
//!
//! Trigonométricas, hiperbólicas, raiz, log, exp (Float → Float),
//! floor/ceil (Float → Int tagged), e aritmética Int (gcd, lcm, pow, signum).

// ── Trigonométricas (Float → Float) ──────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_sin(val: f64) -> f64 {
    val.sin()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_cos(val: f64) -> f64 {
    val.cos()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tan(val: f64) -> f64 {
    val.tan()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_asin(val: f64) -> f64 {
    val.asin()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_acos(val: f64) -> f64 {
    val.acos()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_atan(val: f64) -> f64 {
    val.atan()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_atan2(y: f64, x: f64) -> f64 {
    y.atan2(x)
}

// ── Hiperbólicas (Float → Float) ─────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_sinh(val: f64) -> f64 {
    val.sinh()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_cosh(val: f64) -> f64 {
    val.cosh()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_tanh(val: f64) -> f64 {
    val.tanh()
}

// ── Raiz, log, exp (Float → Float) ───────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_sqrt(val: f64) -> f64 {
    val.sqrt()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_cbrt(val: f64) -> f64 {
    val.cbrt()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_log(val: f64) -> f64 {
    val.ln()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_log2(val: f64) -> f64 {
    val.log2()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_log10(val: f64) -> f64 {
    val.log10()
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_exp(val: f64) -> f64 {
    val.exp()
}

// ── Floor e ceil (Float → Int tagged) ────────────────────────
// Seguem o padrão de kata_rt_float_to_int mas com floor/ceil em vez de trunc.
// NaN/Infinity viram 0.

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_floor(val: f64) -> i64 {
    use crate::bigint::{alloc_bigint, encode_smi_pub, fits_smi_pub};
    use num_bigint::ToBigInt;
    use num_traits::ToPrimitive;

    if !val.is_finite() {
        return encode_smi_pub(0);
    }
    if let Some(big) = val.floor().to_bigint() {
        if let Some(small) = big.to_i64()
            && fits_smi_pub(small)
        {
            return encode_smi_pub(small);
        }
        alloc_bigint(big)
    } else {
        encode_smi_pub(0)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_ceil(val: f64) -> i64 {
    use crate::bigint::{alloc_bigint, encode_smi_pub, fits_smi_pub};
    use num_bigint::ToBigInt;
    use num_traits::ToPrimitive;

    if !val.is_finite() {
        return encode_smi_pub(0);
    }
    if let Some(big) = val.ceil().to_bigint() {
        if let Some(small) = big.to_i64()
            && fits_smi_pub(small)
        {
            return encode_smi_pub(small);
        }
        alloc_bigint(big)
    } else {
        encode_smi_pub(0)
    }
}

// ── Aritmética Int (Int → Int tagged) ────────────────────────
// Recebem SMI-tagged Ints, retornam SMI-tagged Int (ou BigInt se overflow).

/// GCD — máximo divisor comum (algoritmo de Euclides).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_gcd(a: i64, b: i64) -> i64 {
    use crate::bigint::{decode_smi_pub, encode_smi_pub, is_smi_pub};

    let mut x = if is_smi_pub(a) { decode_smi_pub(a) } else { a };
    let mut y = if is_smi_pub(b) { decode_smi_pub(b) } else { b };
    x = x.abs();
    y = y.abs();
    while y != 0 {
        let t = y;
        y = x % y;
        x = t;
    }
    encode_smi_pub(x)
}

/// LCM — mínimo múltiplo comum. Retorna 0 se ambos são 0.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_lcm(a: i64, b: i64) -> i64 {
    use crate::bigint::{decode_smi_pub, encode_smi_pub, is_smi_pub};

    let x = if is_smi_pub(a) { decode_smi_pub(a) } else { a };
    let y = if is_smi_pub(b) { decode_smi_pub(b) } else { b };
    if x == 0 || y == 0 {
        return encode_smi_pub(0);
    }
    let g = gcd_u64(x.unsigned_abs(), y.unsigned_abs());
    let result = (x.unsigned_abs() / g) * y.unsigned_abs();
    // SMI cabe em 63 bits — se overflow, ainda cabe em i64 u64 path
    encode_smi_pub(result as i64)
}

/// GCD em u64 para evitar problemas de overflow no cálculo de LCM.
fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// Exponenciação por quadrado para BigInt.
fn pow_bigint(base: &num_bigint::BigInt, mut exp: usize) -> num_bigint::BigInt {
    use num_traits::One;
    if exp == 0 {
        return num_bigint::BigInt::one();
    }
    let mut result = num_bigint::BigInt::one();
    let mut base = base.clone();
    while exp > 0 {
        if exp & 1 == 1 {
            result *= &base;
        }
        exp >>= 1;
        if exp > 0 {
            base = &base * &base;
        }
    }
    result
}

/// Pow — exponenciação inteira (base^exp). Suporta expoente negativo
/// retornando 0 (inteiros não têm inverso). Usa BigInt se necessário.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_pow(base: i64, exp: i64) -> i64 {
    use crate::bigint::{alloc_bigint, decode_smi_pub, encode_smi_pub, is_smi_pub};
    use num_bigint::BigInt;
    use num_traits::ToPrimitive;

    let b = if is_smi_pub(base) {
        decode_smi_pub(base)
    } else {
        base
    };
    let e = if is_smi_pub(exp) {
        decode_smi_pub(exp)
    } else {
        exp
    };

    if e < 0 {
        return encode_smi_pub(0);
    }
    if e == 0 {
        return encode_smi_pub(1);
    }

    // Tenta fast path com i64 primeiro.
    if let Some(result) = checked_pow_i64(b, e as u64) {
        return encode_smi_pub(result);
    }

    // Overflow — cai para BigInt (exponenciação por quadrado).
    let big_base = BigInt::from(b);
    let result = pow_bigint(&big_base, e as usize);
    if let Some(small) = result.to_i64() {
        return encode_smi_pub(small);
    }
    alloc_bigint(result)
}

/// Exponenciação i64 com checagem de overflow.
fn checked_pow_i64(base: i64, exp: u64) -> Option<i64> {
    if exp == 0 {
        return Some(1);
    }
    if base == 0 {
        return Some(0);
    }
    let mut result: i64 = 1;
    let mut b = base;
    let mut e = exp;
    while e > 0 {
        if e % 2 == 1 {
            result = result.checked_mul(b)?;
        }
        e /= 2;
        if e > 0 {
            b = b.checked_mul(b)?;
        }
    }
    Some(result)
}

/// Signum — retorna -1, 0, ou 1.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_signum(val: i64) -> i64 {
    use crate::bigint::{decode_smi_pub, encode_smi_pub, is_smi_pub};

    let v = if is_smi_pub(val) {
        decode_smi_pub(val)
    } else {
        val
    };
    let s = if v > 0 {
        1
    } else if v < 0 {
        -1
    } else {
        0
    };
    encode_smi_pub(s)
}
