//! Lowering de closures — arm `Closure` do match `lower_expr` + helper `alloc_capture_box`.
//!
//! Extraído de `expr.rs` para reduzir o tamanho do dispatch central.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Signature};
use cranelift_codegen::isa::CallConv;
use kata_core::ty::Ty;
use kata_inference::{CaptureInfo, TypedExpr, TypedExprKind};

use super::LowerCtx;
use crate::ffi_sigs::ty_to_clif;

/// Lowera o arm `TypedExprKind::Closure` — FFI call, call direto (Kata), ou call_indirect.
pub(crate) fn lower_closure(
    expr: &TypedExpr,
    callee: &kata_ast::Spanned<TypedExpr>,
    args: &[kata_ast::Spanned<TypedExpr>],
    ffi_symbol: &Option<String>,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    // Lowera os argumentos.
    let mut arg_values = Vec::with_capacity(args.len());
    for arg in args {
        let val = super::expr::lower_expr(&arg.node, ctx)?;
        arg_values.push(val);
    }

    if let Some(sym_name) = ffi_symbol {
        // Call FFI direto — FFI nunca é tail call (CallConv::SystemV).
        // Mas primeiro tenta kata_refs: funções Kata sintetizadas (ex: repr)
        // usam ffi_symbol para carregar o nome mangled da função.
        if let Some(&func_ref) = ctx.kata_refs.get(sym_name) {
            let call_inst = ctx.builder.ins().call(func_ref, &arg_values);
            return Ok(ctx.builder.inst_results(call_inst)[0]);
        }
        let func_ref = ctx
            .ffi_refs
            .get(sym_name)
            .ok_or_else(|| super::CodegenError::FfiSymbolNotFound(sym_name.clone()))?;
        let call_inst = ctx.builder.ins().call(*func_ref, &arg_values);
        Ok(ctx.builder.inst_results(call_inst)[0])
    } else {
        // ffi_symbol = None: função Kata nomeada ou lambda como valor.
        // Tenta Kata function call direto primeiro.
        if let TypedExprKind::Ident { name } = &callee.node.kind {
            if let Some(&func_ref) = ctx.kata_refs.get(name) {
                // Call direto para função Kata nomeada.
                if expr.tail_pos && !ctx.no_tail_calls {
                    // Tail call: emite return_call (TCO via Cranelift).
                    ctx.builder.ins().return_call(func_ref, &arg_values);
                    ctx.emitted_tail_call = true;
                    // return_call é terminador — não pode adicionar instruções depois.
                    // Criar block dummy unreachable para satisfazer o builder:
                    // todo block precisa de um terminador, mesmo que inalcançável.
                    let dummy = ctx.builder.create_block();
                    ctx.builder.switch_to_block(dummy);
                    ctx.builder.seal_block(dummy);
                    let val = ctx.builder.ins().iconst(I64, 0);
                    ctx.builder.ins().return_(&[val]);
                    return Ok(val);
                }
                let call_inst = ctx.builder.ins().call(func_ref, &arg_values);
                return Ok(ctx.builder.inst_results(call_inst)[0]);
            }
            // Ident não está no kata_refs: pode ser variável com
            // Ty::Function (lambda como valor) — call_indirect.
            if let Some(var) = ctx.var_map.get(name) {
                let func_ptr = ctx.builder.use_var(*var);

                // Se há captures registradas para esta closure,
                // alocar CaptureBox e prefixar box_ptr nos args.
                let caps = ctx.closure_captures.get(name).cloned();
                let mut call_args = Vec::new();
                let mut box_ptr: Option<cranelift_codegen::ir::Value> = None;
                if let Some(ref captures) = caps
                    && !captures.is_empty()
                {
                    let bp = alloc_capture_box(func_ptr, captures, ctx)?;
                    call_args.push(bp);
                    box_ptr = Some(bp);
                }
                call_args.extend(arg_values.iter().copied());

                // Constrói a assinatura para call_indirect.
                // O tipo do callee é Ty::Function(params, ret).
                let callee_ty = &callee.node.ty;
                if let Ty::Function(param_types, ret_ty) = callee_ty {
                    let mut sig = Signature::new(CallConv::Tail);
                    // Se há captures, box_ptr é o primeiro param da sig.
                    if caps.as_ref().is_some_and(|c| !c.is_empty()) {
                        sig.params.push(AbiParam::new(I64));
                    }
                    for pt in param_types {
                        sig.params.push(AbiParam::new(ty_to_clif(pt)));
                    }
                    sig.returns.push(AbiParam::new(ty_to_clif(ret_ty)));
                    let sig_ref = ctx.builder.func.import_signature(sig);
                    if expr.tail_pos && !ctx.no_tail_calls {
                        // Tail call indireto: return_call_indirect.
                        ctx.builder
                            .ins()
                            .return_call_indirect(sig_ref, func_ptr, &call_args);
                        ctx.emitted_tail_call = true;
                        let dummy = ctx.builder.create_block();
                        ctx.builder.switch_to_block(dummy);
                        ctx.builder.seal_block(dummy);
                        let val = ctx.builder.ins().iconst(I64, 0);
                        ctx.builder.ins().return_(&[val]);
                        return Ok(val);
                    }
                    let call_inst = ctx
                        .builder
                        .ins()
                        .call_indirect(sig_ref, func_ptr, &call_args);
                    let result = ctx.builder.inst_results(call_inst)[0];
                    // Pré-11 (Fase 5): ARC pass — decref após call_indirect
                    // de CaptureBox. O refcount volta a 1 (caller terminou
                    // de usar). Não libera memória (bumpalo), mas registra
                    // o padrão correto para GC futuro.
                    if let Some(bp) = box_ptr {
                        let decref_ref =
                            ctx.ffi_refs.get("kata_rt_decref").copied().ok_or_else(|| {
                                super::CodegenError::FfiSymbolNotFound("kata_rt_decref".into())
                            })?;
                        ctx.builder.ins().call(decref_ref, &[bp]);
                    }
                    return Ok(result);
                }
            }
        }
        Err(super::CodegenError::UnsupportedNode(format!(
            "Closure sem ffi_symbol e callee não-Ident: {:?}",
            callee.node.kind
        )))
    }
}

/// Aloca um CaptureBox na arena global e retorna o ponteiro.
///
/// 1. Aloca um array temporário de `n_captures * 8` bytes na arena global.
/// 2. Preenche o array com os valores das captures (lidos do var_map).
/// 3. Chama `kata_rt_alloc_arc(fn_ptr, array_ptr, n_captures)` → `box_ptr`.
///
/// O CaptureBox contém: fn_ptr (offset 0), refcount=1 (offset 8),
/// captures[0..n] (offset 16+).
fn alloc_capture_box(
    func_ptr: cranelift_codegen::ir::Value,
    captures: &[CaptureInfo],
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let n = captures.len() as i64;
    let flags = MemFlagsData::new();

    // 1. Aloca array temporário na arena de escape (Pré-11: caller_arena).
    let arena_alloc_ref = ctx
        .ffi_refs
        .get("kata_rt_arena_alloc")
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_arena_alloc".into()))?;
    let capture_arena = ctx
        .caller_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
    let array_size = ctx.builder.ins().iconst(I64, n * 8);
    let alloc_inst = ctx
        .builder
        .ins()
        .call(*arena_alloc_ref, &[capture_arena, array_size]);
    let array_ptr = ctx.builder.inst_results(alloc_inst)[0];

    // 2. Preenche o array com os valores das captures.
    for (i, cap) in captures.iter().enumerate() {
        let cap_var = ctx.var_map.get(&cap.name).ok_or_else(|| {
            super::CodegenError::UnsupportedNode(format!(
                "capture '{}' não encontrada no var_map",
                cap.name
            ))
        })?;
        let cap_val = ctx.builder.use_var(*cap_var);
        let offset = (i * 8) as i32;
        ctx.builder.ins().store(flags, cap_val, array_ptr, offset);
    }

    // 3. Chama kata_rt_alloc_arc(fn_ptr, array_ptr, n_captures) → box_ptr.
    let alloc_arc_ref = ctx
        .ffi_refs
        .get("kata_rt_alloc_arc")
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_alloc_arc".into()))?;
    let n_val = ctx.builder.ins().iconst(I64, n);
    let arc_inst = ctx
        .builder
        .ins()
        .call(*alloc_arc_ref, &[func_ptr, array_ptr, n_val, capture_arena]);
    let box_ptr = ctx.builder.inst_results(arc_inst)[0];

    Ok(box_ptr)
}
