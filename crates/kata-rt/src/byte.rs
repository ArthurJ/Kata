//! Byte — escalar u8 (SMI-tagged).
//!
//! Operações bitwise e conversões que operam no valor escalar Byte,
//! sem acessar memória de blob. Byte e Int compartilham o mesmo
//! encoding SMI (`(val << 1) | 1`).

use crate::bytes::{tag_smi, untag_smi};

/// AND de dois Bytes (SMI-tagged).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_byte_and(a: i64, b: i64) -> i64 {
    let a = untag_smi(a) as u8;
    let b = untag_smi(b) as u8;
    tag_smi((a & b) as i64)
}

/// OR de dois Bytes (SMI-tagged).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_byte_or(a: i64, b: i64) -> i64 {
    let a = untag_smi(a) as u8;
    let b = untag_smi(b) as u8;
    tag_smi((a | b) as i64)
}

/// XOR de dois Bytes (SMI-tagged).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_byte_xor(a: i64, b: i64) -> i64 {
    let a = untag_smi(a) as u8;
    let b = untag_smi(b) as u8;
    tag_smi((a ^ b) as i64)
}

/// NOT de um Byte (SMI-tagged).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_byte_not(a: i64) -> i64 {
    let a = untag_smi(a) as u8;
    tag_smi((!a) as i64)
}

/// Shift right lógico (Byte, Int) => Byte.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_byte_shr(a: i64, n: i64) -> i64 {
    let a = untag_smi(a) as u8;
    let n = untag_smi(n) as u32;
    tag_smi((a >> n) as i64)
}

/// Shift left (Byte, Int) => Byte.
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_byte_shl(a: i64, n: i64) -> i64 {
    let a = untag_smi(a) as u8;
    let n = untag_smi(n) as u32;
    tag_smi((a << n) as i64)
}

/// Byte → Int (SMI-tagged). Já é SMI, só untag/tag (identity).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_byte_to_int(b: i64) -> i64 {
    b // Byte e Int são ambos SMI-tagged. Identity.
}

/// Int → Byte (SMI-tagged). Trunca para 0-255 (mod 256).
#[unsafe(no_mangle)]
pub extern "C" fn kata_rt_int_to_byte(n: i64) -> i64 {
    let n = untag_smi(n);
    let byte = (n & 0xFF) as u8;
    tag_smi(byte as i64)
}

#[cfg(test)]
#[path = "byte_tests.rs"]
mod tests;
