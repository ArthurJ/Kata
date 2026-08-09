//! Assinaturas FFI para Bytes, Byte, conversões Bytes↔Text e serialização.
//!
//! PRD-bytes: operações binárias, bitwise, slicing, show e conversões de tipos.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use cranelift_codegen::isa::CallConv;
use kata_core::ffi::FfiSymbol;

/// Constrói a assinatura para símbolos de bytes, byte e serialização.
/// Retorna `Some(sig)` se `sym` pertence a esta categoria, `None` caso contrário.
pub(crate) fn sig_for(sym: FfiSymbol) -> Option<Signature> {
    let mut sig = Signature::new(CallConv::SystemV);
    match sym {
        // ── Bytes / Byte (PRD-bytes) ──
        // bytes_alloc: (len, arena) -> ptr
        FfiSymbol::BytesAlloc => {
            sig.params.push(AbiParam::new(I64)); // len
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_from_ptr: (src, len, arena) -> ptr
        FfiSymbol::BytesFromPtr => {
            sig.params.push(AbiParam::new(I64)); // src
            sig.params.push(AbiParam::new(I64)); // len
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_from_ints: (ptrs, count, arena) -> ptr
        FfiSymbol::BytesFromInts => {
            sig.params.push(AbiParam::new(I64)); // ptrs
            sig.params.push(AbiParam::new(I64)); // count
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_len: (ptr) -> i64
        FfiSymbol::BytesLen => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // len
        }
        // bytes_get: (ptr, idx) -> i64
        FfiSymbol::BytesGet => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // bytes_set: (ptr, idx, val) -> void
        FfiSymbol::BytesSet => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.params.push(AbiParam::new(I64)); // val
        }
        // bytes_get_checked: (ptr, idx) -> i64 (Result box)
        FfiSymbol::BytesGetChecked => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // bytes_concat: (a, b, arena) -> ptr
        FfiSymbol::BytesConcat => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_eq: (a, b) -> i64 (0/1)
        FfiSymbol::BytesEq => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // bool
        }
        // bytes_neq: (a, b) -> i64 (0/1)
        FfiSymbol::BytesNeq => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // bool
        }
        // bytes_show: (ptr) -> *mut c_char
        FfiSymbol::BytesShow => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.returns.push(AbiParam::new(I64)); // c_str ptr
        }
        // bytes_slice: (ptr, start, end, arena) -> ptr
        FfiSymbol::BytesSlice => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // start
            sig.params.push(AbiParam::new(I64)); // end
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_and: (a, b, arena) -> ptr
        FfiSymbol::BytesAnd => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_or: (a, b, arena) -> ptr
        FfiSymbol::BytesOr => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_xor: (a, b, arena) -> ptr
        FfiSymbol::BytesXor => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_not: (ptr, arena) -> ptr
        FfiSymbol::BytesNot => {
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // byte_and: (a, b) -> i64
        FfiSymbol::ByteAnd => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // byte_or: (a, b) -> i64
        FfiSymbol::ByteOr => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // byte_xor: (a, b) -> i64
        FfiSymbol::ByteXor => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // byte_not: (a) -> i64
        FfiSymbol::ByteNot => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // byte_shr: (a, n) -> i64
        FfiSymbol::ByteShr => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // n
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // byte_shl: (a, n) -> i64
        FfiSymbol::ByteShl => {
            sig.params.push(AbiParam::new(I64)); // a
            sig.params.push(AbiParam::new(I64)); // n
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // byte_to_int: (b) -> i64
        FfiSymbol::ByteToInt => {
            sig.params.push(AbiParam::new(I64)); // b
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // int_to_byte: (n) -> i64
        FfiSymbol::IntToByte => {
            sig.params.push(AbiParam::new(I64)); // n
            sig.returns.push(AbiParam::new(I64)); // val
        }
        // int_to_bytes: (n, arena) -> ptr
        FfiSymbol::IntToBytes => {
            sig.params.push(AbiParam::new(I64)); // n
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // text_to_bytes: (text_ptr, arena) -> ptr
        FfiSymbol::TextToBytes => {
            sig.params.push(AbiParam::new(I64)); // text_ptr
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        // bytes_to_text: (bytes_ptr) -> *mut c_char
        FfiSymbol::BytesToText => {
            sig.params.push(AbiParam::new(I64)); // bytes_ptr
            sig.returns.push(AbiParam::new(I64)); // c_str ptr
        }
        // text_at: (text_ptr, idx, arena) -> i64 (Result box)
        FfiSymbol::TextAt => {
            sig.params.push(AbiParam::new(I64)); // text_ptr
            sig.params.push(AbiParam::new(I64)); // idx
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // Result box
        }
        // text_len: (text_ptr) -> i64
        FfiSymbol::TextLen => {
            sig.params.push(AbiParam::new(I64)); // text_ptr
            sig.returns.push(AbiParam::new(I64)); // len
        }
        // text_slice: (text_ptr, start, end) -> *mut c_char
        FfiSymbol::TextSlice => {
            sig.params.push(AbiParam::new(I64)); // text_ptr
            sig.params.push(AbiParam::new(I64)); // start
            sig.params.push(AbiParam::new(I64)); // end
            sig.returns.push(AbiParam::new(I64)); // c_str ptr
        }
        // to_bytes: (value_ptr, type_id, arena) -> bytes_ptr
        FfiSymbol::ToBytes => {
            sig.params.push(AbiParam::new(I64)); // value_ptr
            sig.params.push(AbiParam::new(I64)); // type_id
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // bytes_ptr
        }
        // from_bytes: (bytes_ptr, arena) -> value_ptr
        FfiSymbol::FromBytes => {
            sig.params.push(AbiParam::new(I64)); // bytes_ptr
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // value_ptr
        }
        _ => return None,
    }
    Some(sig)
}
