//! Helper centralizado para seleção de arena baseada em `EscapeTarget`.
//!
//! `Local` → fiber_arena, `Caller` → caller_arena, `Heap` → root_arena
//! (via `kata_rt_get_root_arena_handle` FFI).

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::InstBuilder;
use kata_core::escape::EscapeTarget;

use super::LowerCtx;

/// Seleciona o handle de arena conforme `escape`:
/// - `Local` → `fiber_arena` (fallback `iconst 0`)
/// - `Caller` → `caller_arena` (fallback `iconst 0`)
/// - `Heap` → `root_arena` (via `kata_rt_get_root_arena_handle` FFI)
pub(crate) fn arena_handle_for_escape(
    escape: EscapeTarget,
    ctx: &mut LowerCtx,
) -> cranelift_codegen::ir::Value {
    match escape {
        EscapeTarget::Local => ctx
            .fiber_arena
            .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
        EscapeTarget::Caller => ctx
            .caller_arena
            .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
        EscapeTarget::Heap => {
            let get_root_ref = ctx
                .ffi_refs
                .get("kata_rt_get_root_arena_handle")
                .copied()
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_get_root_arena_handle".into())
                })
                .expect("kata_rt_get_root_arena_handle must be registered");
            let root_inst = ctx.builder.ins().call(get_root_ref, &[]);
            ctx.builder.inst_results(root_inst)[0]
        }
    }
}