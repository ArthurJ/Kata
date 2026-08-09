//! Assinaturas FFI para I/O simples, controle de fluxo e timer.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use cranelift_codegen::isa::CallConv;
use kata_core::ffi::FfiSymbol;

/// Constrói a assinatura para símbolos de I/O, panic e timer.
/// Retorna `Some(sig)` se `sym` pertence a esta categoria, `None` caso contrário.
pub(crate) fn sig_for(sym: FfiSymbol) -> Option<Signature> {
    let mut sig = Signature::new(CallConv::SystemV);
    match sym {
        // ── I/O (ptr) → void ──
        FfiSymbol::Print | FfiSymbol::Println => {
            sig.params.push(AbiParam::new(I64));
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
