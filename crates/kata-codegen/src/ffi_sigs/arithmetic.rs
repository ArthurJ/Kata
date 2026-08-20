//! Assinaturas FFI para aritmética, comparação e conversões de tipos primitivos.
//!
//! Int, Float, Rational, Text (concat/len/literal/show), Boolean e StringEq.

use crate::call_conv::ffi_call_conv;
use cranelift_codegen::ir::types::{F64, I64};
use cranelift_codegen::ir::{AbiParam, Signature};
use kata_core::ffi::FfiSymbol;

/// Constrói a assinatura para símbolos de aritmética/comparação/conversão.
/// Retorna `Some(sig)` se `sym` pertence a esta categoria, `None` caso contrário.
pub(crate) fn sig_for(sym: FfiSymbol) -> Option<Signature> {
    let mut sig = Signature::new(ffi_call_conv());
    match sym {
        // ── Aritmética Int (i64, i64) → i64 ──
        FfiSymbol::BiAdd | FfiSymbol::BiSub | FfiSymbol::BiMul | FfiSymbol::BiDiv => {
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Comparação Int (i64, i64) → i64 (0/1) ──
        FfiSymbol::BiEq
        | FfiSymbol::BiNeq
        | FfiSymbol::BiLt
        | FfiSymbol::BiLe
        | FfiSymbol::BiGt
        | FfiSymbol::BiGe => {
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Show Int → Text (i64) → i64 (ptr) ──
        FfiSymbol::BiShow => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Int → Rational (i64) → i64 (ptr) ──
        FfiSymbol::BiToRational => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Tagging: i64 cru → i64 SMI/BigInt ──
        FfiSymbol::TagInt => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Aritmética Float (f64, f64) → f64 ──
        FfiSymbol::Fadd | FfiSymbol::Fsub | FfiSymbol::Fmul | FfiSymbol::Fdiv => {
            sig.params.push(AbiParam::new(F64));
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(F64));
        }
        // ── Comparação Float (f64, f64) → i64 (0/1) ──
        FfiSymbol::FcmpEq
        | FfiSymbol::FcmpNeq
        | FfiSymbol::FcmpLt
        | FfiSymbol::FcmpLe
        | FfiSymbol::FcmpGt
        | FfiSymbol::FcmpGe => {
            sig.params.push(AbiParam::new(F64));
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Float → Text (f64) → i64 (ptr) ──
        FfiSymbol::FloatToText => {
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Aritmética Rational (ptr, ptr) → ptr ──
        FfiSymbol::RatAdd | FfiSymbol::RatSub | FfiSymbol::RatMul | FfiSymbol::RatDiv => {
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Comparação Rational (ptr, ptr) → i64 (0/1) ──
        FfiSymbol::RatEq
        | FfiSymbol::RatNeq
        | FfiSymbol::RatLt
        | FfiSymbol::RatLe
        | FfiSymbol::RatGt
        | FfiSymbol::RatGe => {
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Show Rational → Text (ptr) → i64 (ptr) ──
        FfiSymbol::RatShow => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Rational → Float (ptr) → f64 ──
        FfiSymbol::RatToFloat => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(F64));
        }
        // ── Float → Rational (f64) → i64 (ptr) ──
        FfiSymbol::RatFromFloat => {
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Literal Rational (ptr texto, len) → ptr ──
        // RatLiteral recebe ponteiro para string C + length.
        FfiSymbol::RatLiteral => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // len
            sig.returns.push(AbiParam::new(I64)); // ptr Rational
        }
        // ── Int → Rational (i64 tagged) → i64 (ptr) ──
        FfiSymbol::IntToRational => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Int → Float (i64 tagged) → f64 ──
        FfiSymbol::IntToFloat => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(F64));
        }
        // ── Float → Int (f64) → i64 tagged ──
        FfiSymbol::FloatToInt => {
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Rational → Int (ptr) → i64 tagged ──
        FfiSymbol::RatToInt => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Text concat (ptr, ptr) → ptr ──
        FfiSymbol::StringConcat => {
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Text len (ptr) → i64 ──
        FfiSymbol::StringLen => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Text literal (ptr, len) → ptr ──
        FfiSymbol::TextLiteral => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // len
            sig.returns.push(AbiParam::new(I64)); // ptr string
        }
        // ── Int → Text (i64 tagged) → ptr ──
        FfiSymbol::IntToText => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Text (ptr C string) → Int (i64 tagged) ──
        FfiSymbol::TextToInt => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // tagged Int
        }
        // ── Text (ptr C string) → Float (f64) ──
        FfiSymbol::TextToFloat => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(F64)); // f64
        }
        // ── Rand () → Float (f64) ──
        FfiSymbol::Rand => {
            sig.returns.push(AbiParam::new(F64)); // f64
        }
        // ── RandInt (min, max) → Int (i64 SMI-tagged) ──
        FfiSymbol::RandInt => {
            sig.params.push(AbiParam::new(I64)); // min
            sig.params.push(AbiParam::new(I64)); // max
            sig.returns.push(AbiParam::new(I64)); // tagged Int
        }
        // ── Boolean → Text (i64 0/1) → ptr ──
        FfiSymbol::BoolToText => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Text replace first (ptr, ptr) → ptr ──
        FfiSymbol::TextReplaceFirst => {
            sig.params.push(AbiParam::new(I64)); // template
            sig.params.push(AbiParam::new(I64)); // replacement
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Text replace (ptr, ptr, ptr) → ptr — substitui needle por replacement ──
        FfiSymbol::TextReplace => {
            sig.params.push(AbiParam::new(I64)); // template
            sig.params.push(AbiParam::new(I64)); // needle
            sig.params.push(AbiParam::new(I64)); // replacement
            sig.returns.push(AbiParam::new(I64));
        }
        // ── String comparison ( + expects) ──
        // string_eq: (a, b) -> i64 (0/1)
        // string_starts_with: (haystack, needle) -> i64 (0/1)
        // string_contains: (haystack, needle) -> i64 (0/1)
        FfiSymbol::StringEq | FfiSymbol::StringStartsWith | FfiSymbol::StringContains => {
            sig.params.push(AbiParam::new(I64)); // haystack/a (str ptr)
            sig.params.push(AbiParam::new(I64)); // needle/b (str ptr)
            sig.returns.push(AbiParam::new(I64)); // bool (0/1)
        }
        // ── Math: Float → Float (f64) → f64 ──
        FfiSymbol::Sin
        | FfiSymbol::Cos
        | FfiSymbol::Tan
        | FfiSymbol::Asin
        | FfiSymbol::Acos
        | FfiSymbol::Atan
        | FfiSymbol::Sinh
        | FfiSymbol::Cosh
        | FfiSymbol::Tanh
        | FfiSymbol::Sqrt
        | FfiSymbol::Cbrt
        | FfiSymbol::Log
        | FfiSymbol::Log2
        | FfiSymbol::Log10
        | FfiSymbol::Exp => {
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(F64));
        }
        // ── Math: atan2 (f64, f64) → f64 ──
        FfiSymbol::Atan2 => {
            sig.params.push(AbiParam::new(F64));
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(F64));
        }
        // ── Math: floor/ceil (f64) → i64 tagged ──
        FfiSymbol::Floor | FfiSymbol::Ceil => {
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Math: gcd/lcm/pow (i64, i64) → i64 tagged ──
        FfiSymbol::Gcd | FfiSymbol::Lcm | FfiSymbol::Pow => {
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        // ── Math: signum (i64) → i64 tagged ──
        FfiSymbol::Signum => {
            sig.params.push(AbiParam::new(I64));
            sig.returns.push(AbiParam::new(I64));
        }
        _ => return None,
    }
    Some(sig)
}
