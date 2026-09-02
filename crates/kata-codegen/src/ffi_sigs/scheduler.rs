//! Assinaturas FFI para Scheduler/Fiber, Arc<T>/CaptureBox e spawn de processos.

use crate::call_conv::ffi_call_conv;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use kata_core::ffi::FfiSymbol;

/// Constrói a assinatura para símbolos de scheduler, fiber, arc e processos.
/// Retorna `Some(sig)` se `sym` pertence a esta categoria, `None` caso contrário.
pub(crate) fn sig_for(sym: FfiSymbol) -> Option<Signature> {
    let mut sig = Signature::new(ffi_call_conv());
    match sym {
        // ── Scheduler/Fiber ──
        // A2: scheduler_init: (rt: i64) -> i64 (root_arena_handle)
        FfiSymbol::SchedulerInit => {
            sig.params.push(AbiParam::new(I64)); // rt
            sig.returns.push(AbiParam::new(I64)); // root_arena_handle
        }
        // A2: spawn: (rt: i64, fn_ptr: i64, caller_arena: i64, args_ptr: i64) -> i64 (fiber_id)
        FfiSymbol::Spawn => {
            sig.params.push(AbiParam::new(I64)); // rt
            sig.params.push(AbiParam::new(I64)); // fn_ptr
            sig.params.push(AbiParam::new(I64)); // caller_arena
            sig.params.push(AbiParam::new(I64)); // args_ptr
            sig.returns.push(AbiParam::new(I64)); // fiber_id
        }
        // A2: run: (rt: i64) -> i64 (resultado do fiber)
        FfiSymbol::Run => {
            sig.params.push(AbiParam::new(I64)); // rt
            sig.returns.push(AbiParam::new(I64)); // resultado
        }
        // yield: () → void (suspende fiber)
        FfiSymbol::Yield => {}
        // A2: yield_check: (rt: i64) → void (yield point no header de loops)
        FfiSymbol::YieldCheck => {
            sig.params.push(AbiParam::new(I64)); // rt
        }
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
        // A2: alloc_arc: (rt, fn_ptr, captures_ptr, n_captures, arena_handle) -> box_ptr
        FfiSymbol::AllocArc => {
            sig.params.push(AbiParam::new(I64)); // rt
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
        // A2: decref: (rt, box_ptr) -> 0
        FfiSymbol::DecRef => {
            sig.params.push(AbiParam::new(I64)); // rt
            sig.params.push(AbiParam::new(I64)); // box_ptr
            sig.returns.push(AbiParam::new(I64));
        }
        // arc_fn_ptr: (box_ptr) -> fn_ptr
        FfiSymbol::ArcFnPtr => {
            sig.params.push(AbiParam::new(I64)); // box_ptr
            sig.returns.push(AbiParam::new(I64)); // fn_ptr
        }
        // A2: spawn_process: (rt, fn_ptr, args_ptr, arena) -> void (fire-and-forget)
        FfiSymbol::SpawnProcess => {
            sig.params.push(AbiParam::new(I64)); // rt
            sig.params.push(AbiParam::new(I64)); // fn_ptr
            sig.params.push(AbiParam::new(I64)); // args_ptr
            sig.params.push(AbiParam::new(I64)); // arena
        }
        // ── Recursion depth ──
        // depth_inc: (rt) -> i64 (nova profundidade)
        FfiSymbol::DepthInc => {
            sig.params.push(AbiParam::new(I64)); // rt
            sig.returns.push(AbiParam::new(I64)); // depth
        }
        // depth_dec: (rt) -> void
        FfiSymbol::DepthDec => {
            sig.params.push(AbiParam::new(I64)); // rt
        }
        // depth_get: (rt) -> i64 (profundidade atual)
        FfiSymbol::DepthGet => {
            sig.params.push(AbiParam::new(I64)); // rt
            sig.returns.push(AbiParam::new(I64)); // depth
        }
        // depth_set_limit: (rt, limit) -> void
        FfiSymbol::DepthSetLimit => {
            sig.params.push(AbiParam::new(I64)); // rt
            sig.params.push(AbiParam::new(I64)); // limit
        }
        // set_overflowed: (rt) -> void
        FfiSymbol::SetOverflowed => {
            sig.params.push(AbiParam::new(I64)); // rt
        }
        // overflowed: (rt) -> i64 (1=overflow, 0=ok)
        FfiSymbol::Overflowed => {
            sig.params.push(AbiParam::new(I64)); // rt
            sig.returns.push(AbiParam::new(I64)); // bool as i64
        }
        // depth_get_limit: (rt) -> i64 (limite)
        FfiSymbol::DepthGetLimit => {
            sig.params.push(AbiParam::new(I64)); // rt
            sig.returns.push(AbiParam::new(I64)); // limit
        }
        // reset_depth: (rt) -> void
        FfiSymbol::ResetDepth => {
            sig.params.push(AbiParam::new(I64)); // rt
        }
        _ => return None,
    }
    Some(sig)
}
