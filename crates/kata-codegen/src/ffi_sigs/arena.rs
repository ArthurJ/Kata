//! Assinaturas FFI para Arena e Sum (box tag+payload).

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use cranelift_codegen::isa::CallConv;
use kata_core::ffi::FfiSymbol;

/// Constrói a assinatura para símbolos de Arena e Sum.
/// Retorna `Some(sig)` se `sym` pertence a esta categoria, `None` caso contrário.
pub(crate) fn sig_for(sym: FfiSymbol) -> Option<Signature> {
    let mut sig = Signature::new(CallConv::SystemV);
    match sym {
        // ── Arena (void → ptr, ptr,size → ptr, ptr → void) ──
        FfiSymbol::ArenaCreate => {
            sig.returns.push(AbiParam::new(I64));
        }
        FfiSymbol::ArenaAlloc => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.params.push(AbiParam::new(I64)); // size
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        FfiSymbol::ArenaDestroy => {
            sig.params.push(AbiParam::new(I64)); // arena
        }
        FfiSymbol::ArenaCreateTracked => {
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        FfiSymbol::ArenaDealloc => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // ptr
            sig.params.push(AbiParam::new(I64)); // size
        }
        FfiSymbol::GetRootArenaHandle => {
            sig.returns.push(AbiParam::new(I64)); // root_arena handle
        }
        FfiSymbol::ArenaStats => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.returns.push(AbiParam::new(I64)); // packed (alloc_count, dealloc_count)
        }
        // ── Sum (i64, i64, i64) → i64, (i64) → i64 ──
        // Pré-11: store_sum_result recebe arena_handle como 3º param.
        FfiSymbol::StoreSumResult => {
            sig.params.push(AbiParam::new(I64)); // tag
            sig.params.push(AbiParam::new(I64)); // payload
            sig.params.push(AbiParam::new(I64)); // arena_handle
            sig.returns.push(AbiParam::new(I64)); // ptr
        }
        FfiSymbol::SumTagInt => {
            sig.params.push(AbiParam::new(I64)); // val (ptr to box)
            sig.returns.push(AbiParam::new(I64)); // tag
        }
        _ => return None,
    }
    Some(sig)
}
