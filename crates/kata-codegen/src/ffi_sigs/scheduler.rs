//! Assinaturas FFI para Scheduler/Fiber, Arc<T>/CaptureBox e spawn de processos.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use cranelift_codegen::isa::CallConv;
use kata_core::ffi::FfiSymbol;

/// Constrói a assinatura para símbolos de scheduler, fiber, arc e processos.
/// Retorna `Some(sig)` se `sym` pertence a esta categoria, `None` caso contrário.
pub(crate) fn sig_for(sym: FfiSymbol) -> Option<Signature> {
    let mut sig = Signature::new(CallConv::SystemV);
    match sym {
        // ── Scheduler/Fiber ──
        // scheduler_init: () -> i64 (1 = sucesso)
        FfiSymbol::SchedulerInit => {
            sig.returns.push(AbiParam::new(I64));
        }
        // spawn: (fn_ptr: i64, caller_arena: i64, args_ptr: i64) -> i64 (fiber_id)
        FfiSymbol::Spawn => {
            sig.params.push(AbiParam::new(I64)); // fn_ptr
            sig.params.push(AbiParam::new(I64)); // caller_arena
            sig.params.push(AbiParam::new(I64)); // args_ptr
            sig.returns.push(AbiParam::new(I64)); // fiber_id
        }
        // run: () -> i64 (resultado do fiber)
        FfiSymbol::Run => {
            sig.returns.push(AbiParam::new(I64));
        }
        // yield: () → void (suspende fiber)
        FfiSymbol::Yield => {}
        // yield_check: () → void (yield point no header de loops, )
        FfiSymbol::YieldCheck => {}
        // set_test_timeout: (millis: i64) → void (configura timer de teste)
        // Chamada pelo runner antes de kata_rt_run.
        FfiSymbol::SetTestTimeout => {
            sig.params.push(AbiParam::new(I64)); // millis
        }
        // sleep: (ms: i64) → void (sleep cooperativo, suspende fiber)
        FfiSymbol::Sleep => {
            sig.params.push(AbiParam::new(I64)); // ms (SMI-tagged)
        }
        // ── Arc<T> / CaptureBox ──
        // alloc_arc: (fn_ptr, captures_ptr, n_captures, arena_handle) -> box_ptr
        // Pré-11: arena_handle adicionado como 4º param.
        FfiSymbol::AllocArc => {
            sig.params.push(AbiParam::new(I64)); // fn_ptr
            sig.params.push(AbiParam::new(I64)); // captures_ptr
            sig.params.push(AbiParam::new(I64)); // n_captures
            sig.params.push(AbiParam::new(I64)); // arena_handle
            sig.returns.push(AbiParam::new(I64)); // box_ptr
        }
        // incref: (box_ptr) -> 0
        FfiSymbol::IncRef => {
            sig.params.push(AbiParam::new(I64)); // box_ptr
            sig.returns.push(AbiParam::new(I64));
        }
        // decref: (box_ptr) -> 0
        FfiSymbol::DecRef => {
            sig.params.push(AbiParam::new(I64)); // box_ptr
            sig.returns.push(AbiParam::new(I64));
        }
        // arc_fn_ptr: (box_ptr) -> fn_ptr
        FfiSymbol::ArcFnPtr => {
            sig.params.push(AbiParam::new(I64)); // box_ptr
            sig.returns.push(AbiParam::new(I64)); // fn_ptr
        }
        // spawn_process: (fn_ptr, args_ptr, arena) -> void (fire-and-forget)
        FfiSymbol::SpawnProcess => {
            sig.params.push(AbiParam::new(I64)); // fn_ptr
            sig.params.push(AbiParam::new(I64)); // args_ptr
            sig.params.push(AbiParam::new(I64)); // arena
        }
        _ => return None,
    }
    Some(sig)
}
