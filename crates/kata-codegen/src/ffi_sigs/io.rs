//! Assinaturas FFI para I/O simples, controle de fluxo e timer.

use crate::call_conv::ffi_call_conv;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use kata_core::ffi::FfiSymbol;

/// Constrói a assinatura para símbolos de I/O, panic e timer.
/// Retorna `Some(sig)` se `sym` pertence a esta categoria, `None` caso contrário.
pub(crate) fn sig_for(sym: FfiSymbol) -> Option<Signature> {
    let mut sig = Signature::new(ffi_call_conv());
    match sym {
        // ── I/O (ptr) → void ──
        FfiSymbol::Print | FfiSymbol::Println => {
            sig.params.push(AbiParam::new(I64));
        }
        // input: (prompt_ptr) -> i64 (Text ptr)
        FfiSymbol::Input => {
            sig.params.push(AbiParam::new(I64)); // prompt_ptr (Text — C string)
            sig.returns.push(AbiParam::new(I64)); // Text ptr
        }
        // try_int: (text_ptr) -> i64 (Result box ptr)
        FfiSymbol::TryInt => {
            sig.params.push(AbiParam::new(I64)); // text_ptr (Text — C string)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // try_float: (text_ptr) -> i64 (Result box ptr)
        FfiSymbol::TryFloat => {
            sig.params.push(AbiParam::new(I64)); // text_ptr (Text — C string)
            sig.returns.push(AbiParam::new(I64)); // Result box ptr
        }
        // ── Control flow (ptr) → void (never returns) ──
        FfiSymbol::Panic => {
            sig.params.push(AbiParam::new(I64)); // msg ptr
        }
        // ── Timer ──
        // timer_now: () -> i64 (nanossegundos do clock monotônico)
        FfiSymbol::TimerNow => {
            sig.returns.push(AbiParam::new(I64)); // nanossegundos
        }
        _ => return None,
    }
    Some(sig)
}
