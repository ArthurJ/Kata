//! Helper centralizado para seleção de arena baseada em `EscapeTarget`.
//!
//! `Local` → fiber_arena, `Caller` → caller_arena, `Heap` → root_arena
//! (via `kata_rt_get_root_arena_handle` FFI).
//!
//! Para `Heap`, a alocação usa `kata_rt_alloc_tracked` que prefixa um header
//! ARC {refcount, size, destructor} antes dos dados. O caller recebe
//! `data_ptr` (ponteiro para os dados, pulando o header).

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
                    super::CodegenError::FfiSymbolNotFound("kata_rt_get_root_arena_handle".into())
                })
                .expect("kata_rt_get_root_arena_handle must be registered");
            let root_inst = ctx.builder.ins().call(get_root_ref, &[]);
            ctx.builder.inst_results(root_inst)[0]
        }
    }
}

/// Aloca `data_size` bytes na arena apropriada para `escape`.
///
/// - `Local`/`Caller`: chama `kata_rt_arena_alloc(handle, size)` → `ptr`
/// - `Heap`: chama `kata_rt_alloc_tracked(root_arena, data_size, 0)` → `data_ptr`
///   (destructor = 0 = leaf; para estruturas recursivas, futuro)
///
/// Retorna o ponteiro para os dados (em `Heap`, já pulando o header ARC).
pub(crate) fn alloc_for_escape(
    escape: EscapeTarget,
    data_size: i64,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let size_val = ctx.builder.ins().iconst(I64, data_size);

    match escape {
        EscapeTarget::Heap => {
            // Aloca na root_arena com header ARC.
            // destructor = 0 (leaf) por enquanto.
            let get_root_ref = ctx
                .ffi_refs
                .get("kata_rt_get_root_arena_handle")
                .copied()
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_get_root_arena_handle".into())
                })?;
            let root_inst = ctx.builder.ins().call(get_root_ref, &[]);
            let root_arena = ctx.builder.inst_results(root_inst)[0];

            let alloc_ref = ctx
                .ffi_refs
                .get("kata_rt_alloc_tracked")
                .copied()
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_alloc_tracked".into())
                })?;
            let destructor = ctx.builder.ins().iconst(I64, 0); // leaf
            let inst = ctx
                .builder
                .ins()
                .call(alloc_ref, &[root_arena, size_val, destructor]);
            Ok(ctx.builder.inst_results(inst)[0])
        }
        _ => {
            // Alocação normal em fiber/caller arena.
            let handle = arena_handle_for_escape(escape, ctx);
            let alloc_ref = ctx
                .ffi_refs
                .get("kata_rt_arena_alloc")
                .copied()
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_arena_alloc".into())
                })?;
            let inst = ctx.builder.ins().call(alloc_ref, &[handle, size_val]);
            Ok(ctx.builder.inst_results(inst)[0])
        }
    }
}

/// Emite `incref_tracked(data_ptr)` se `escape == Heap`, no-op caso contrário.
pub(crate) fn incref_if_heap(
    escape: EscapeTarget,
    data_ptr: cranelift_codegen::ir::Value,
    ctx: &mut LowerCtx,
) -> Result<(), super::CodegenError> {
    if escape == EscapeTarget::Heap {
        let incref_ref = ctx
            .ffi_refs
            .get("kata_rt_incref_tracked")
            .copied()
            .ok_or_else(|| {
                super::CodegenError::FfiSymbolNotFound("kata_rt_incref_tracked".into())
            })?;
        ctx.builder.ins().call(incref_ref, &[data_ptr]);
    }
    Ok(())
}

/// Emite `decref_tracked(data_ptr)` se `escape == Heap`, no-op caso contrário.
pub(crate) fn decref_if_heap(
    escape: EscapeTarget,
    data_ptr: cranelift_codegen::ir::Value,
    ctx: &mut LowerCtx,
) -> Result<(), super::CodegenError> {
    if escape == EscapeTarget::Heap {
        let decref_ref = ctx
            .ffi_refs
            .get("kata_rt_decref_tracked")
            .copied()
            .ok_or_else(|| {
                super::CodegenError::FfiSymbolNotFound("kata_rt_decref_tracked".into())
            })?;
        ctx.builder.ins().call(decref_ref, &[data_ptr]);
    }
    Ok(())
}
