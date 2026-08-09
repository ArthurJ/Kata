//! Helper centralizado para seleção de arena baseada em `EscapeTarget`.
//!
//! `Local` → fiber_arena, `Caller`/`Heap` → caller_arena (ou root_arena
//! via `kata_rt_get_root_arena_handle` se caller_arena não disponível).
//!
//! Não há mais distinção entre `Heap` e `Caller` na alocação — ambas
//! usam `kata_rt_arena_alloc` (sem header ARC). O modelo de memória
//! atual usa arenas bump para todos os valores, com cleanup automático
//! quando a arena é resetada. File handles recevem close explícito
//! (ou automático no epílogo da action).

use cranelift_codegen::ir::InstBuilder;
use cranelift_codegen::ir::types::I64;
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
                    super::CodegenError::FfiSymbolNotFound { symbol: "kata_rt_get_root_arena_handle".into() }
                })
                .expect("kata_rt_get_root_arena_handle must be registered");
            let root_inst = ctx.builder.ins().call(get_root_ref, &[]);
            ctx.builder.inst_results(root_inst)[0]
        }
    }
}

/// Aloca `data_size` bytes na arena apropriada para `escape`.
///
/// Todos os paths usam `kata_rt_arena_alloc` (sem header ARC).
/// Retorna o ponteiro para os dados.
pub(crate) fn alloc_for_escape(
    escape: EscapeTarget,
    data_size: i64,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let size_val = ctx.builder.ins().iconst(I64, data_size);
    let handle = arena_handle_for_escape(escape, ctx);
    let alloc_ref = ctx
        .ffi_refs
        .get("kata_rt_arena_alloc")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound { symbol: "kata_rt_arena_alloc".into() })?;
    let inst = ctx.builder.ins().call(alloc_ref, &[handle, size_val]);
    Ok(ctx.builder.inst_results(inst)[0])
}
