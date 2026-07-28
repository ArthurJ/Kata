//! Lowering de map/filter/fold — higher-order sobre coleções.
//!
//! Estas funções são interceptadas no typeck (infer_apply) e produzidas
//! como nós TAST dedicados (Map/Filter/Fold). O codegen percorre a coleção
//! por tipo concreto (List/Array/Range), chama o callback via call_indirect,
//! e constrói o resultado.
//!
//! **Map/Filter:** constroem a lista de resultado com `cons` (prepend),
//! que inverte a ordem. No final, chamam `kata_rt_list_reverse` para
//! restaurar a ordem original. Se o input era Array, converte List→Array.
//!
//! **Fold:** percorre a coleção acumulando `acc = f(acc, elem)`. Não
//! constrói coleção — retorna o acumulador.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Signature};
use cranelift_codegen::isa::CallConv;
use kata_core::ty::Ty;
use kata_inference::TypedExpr;

use super::CodegenError;
use super::LowerCtx;

// ── Helpers ──────────────────────────────────────────────────

/// Bitcast F64→I64 se necessário (elementos Float são armazenados como I64).
pub(crate) fn ensure_i64(
    ctx: &mut LowerCtx,
    val: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let ty = ctx.builder.func.dfg.value_type(val);
    if ty == cranelift_codegen::ir::types::F64 {
        ctx.builder.ins().bitcast(I64, MemFlagsData::new(), val)
    } else {
        val
    }
}

/// Bitcast I64→F64 se o tipo alvo é Float.
pub(crate) fn ensure_f64_if(
    ctx: &mut LowerCtx,
    val: cranelift_codegen::ir::Value,
    target_ty: &Ty,
) -> cranelift_codegen::ir::Value {
    if *target_ty == Ty::float() {
        ctx.builder
            .ins()
            .bitcast(cranelift_codegen::ir::types::F64, MemFlagsData::new(), val)
    } else {
        val
    }
}

/// Arena handle para alocação de Cons cells (fiber_arena ou fallback 0).
pub(crate) fn arena_handle(ctx: &mut LowerCtx) -> cranelift_codegen::ir::Value {
    ctx.fiber_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0))
}

/// Constrói a assinatura Cranelift para um callback com tipos de parâmetros
/// e retorno conhecidos. Se o callback tem captures, o segundo param é I64.
/// O primeiro param é sempre arena_handle (I64) — param implícito.
fn build_callback_sig(
    param_types: &[Ty],
    ret_ty: &Ty,
    _has_captures: bool,
    struct_registry: &kata_core::StructRegistry,
) -> Signature {
    let mut sig = Signature::new(CallConv::Tail);
    sig.params.push(AbiParam::new(I64)); // arena_handle
    sig.params.push(AbiParam::new(I64)); // box_ptr (sempre presente na ABI uniformizada)
    for pt in param_types {
        sig.params
            .push(AbiParam::new(super::resolve_clif_ty(pt, struct_registry)));
    }
    sig.returns.push(AbiParam::new(super::resolve_clif_ty(
        ret_ty,
        struct_registry,
    )));
    sig
}

/// Chama um callback (func_ptr) com os argumentos fornecidos.
/// `callback_val` é o function pointer (I64).
/// `param_types` e `ret_ty` definem a assinatura.
/// Retorna o resultado do callback (Value).
pub(crate) fn call_callback(
    callback_val: cranelift_codegen::ir::Value,
    args: &[cranelift_codegen::ir::Value],
    param_types: &[Ty],
    ret_ty: &Ty,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    // ABI uniformizada: callback_val é box_ptr (CaptureBox).
    // Carrega fn_ptr do offset 0 e passa box_ptr como 2º param.
    let flags = MemFlagsData::new();
    let func_ptr = ctx.builder.ins().load(I64, flags, callback_val, 0);

    let sig = build_callback_sig(param_types, ret_ty, true, ctx.struct_registry);
    let sig_ref = ctx.builder.func.import_signature(sig);
    let arena = ctx
        .fiber_arena
        .or(ctx.caller_arena)
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
    let mut full_args = vec![arena, callback_val]; // arena_handle, box_ptr
    full_args.extend_from_slice(args);
    let call_inst = ctx
        .builder
        .ins()
        .call_indirect(sig_ref, func_ptr, &full_args);
    Ok(ctx.builder.inst_results(call_inst)[0])
}

/// Extrai tipos do callback (params, ret) a partir do tipo do callback.
pub(crate) fn extract_callback_sig(callback: &TypedExpr) -> (Vec<Ty>, Ty) {
    match &callback.ty {
        Ty::Function(params, ret) => (params.clone(), (**ret).clone()),
        _ => panic!("callback não é Function: {}", callback.ty),
    }
}

// ── Fold ─────────────────────────────────────────────────────

pub(crate) fn lower_fold(
    callback: &kata_ast::Spanned<TypedExpr>,
    initial: &kata_ast::Spanned<TypedExpr>,
    collection: &kata_ast::Spanned<TypedExpr>,
    coll_ty: &Ty,
    elem_ty: &Ty,
    ret_ty: &Ty,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    let coll_val = super::expr::lower_expr(&collection.node, ctx)?;
    let callback_val = super::expr::lower_expr(&callback.node, ctx)?;
    let init_val = super::expr::lower_expr(&initial.node, ctx)?;
    let init_val = ensure_i64(ctx, init_val);

    let (cb_params, cb_ret) = extract_callback_sig(&callback.node);

    // acc = init
    let acc_var = ctx.new_var("__fold_acc", I64);
    ctx.builder.def_var(acc_var, init_val);

    let loop_block = ctx.builder.create_block();
    let continue_block = ctx.builder.create_block();
    let break_block = ctx.builder.create_block();

    match coll_ty {
        Ty::List(_) => {
            let current_var = ctx.new_var("__fold_current", I64);
            ctx.builder.def_var(current_var, coll_val);

            ctx.builder.ins().jump(loop_block, &[]);
            ctx.builder.switch_to_block(loop_block);
            let current = ctx.builder.use_var(current_var);
            let is_nil = ctx.builder.ins().icmp_imm(
                cranelift_codegen::ir::condcodes::IntCC::Equal,
                current,
                0,
            );
            ctx.builder
                .ins()
                .brif(is_nil, break_block, &[], continue_block, &[]);

            ctx.builder.switch_to_block(continue_block);
            let flags = MemFlagsData::new();
            let head_val = ctx.builder.ins().load(I64, flags, current, 0);
            let head_val = ensure_f64_if(ctx, head_val, elem_ty);
            let tail_val = ctx.builder.ins().load(I64, flags, current, 8);

            // acc = callback(acc, elem)
            let acc = ctx.builder.use_var(acc_var);
            let new_acc = call_callback(callback_val, &[acc, head_val], &cb_params, &cb_ret, ctx)?;
            let new_acc = ensure_i64(ctx, new_acc);
            ctx.builder.def_var(acc_var, new_acc);
            ctx.builder.def_var(current_var, tail_val);
            ctx.builder.ins().jump(loop_block, &[]);

            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);
        }
        Ty::Array(_) => {
            let flags = MemFlagsData::new();
            let len_val = ctx.builder.ins().load(I64, flags, coll_val, 0);
            let idx_var = ctx.new_var("__fold_idx", I64);
            let zero = ctx.builder.ins().iconst(I64, 0);
            ctx.builder.def_var(idx_var, zero);

            ctx.builder.ins().jump(loop_block, &[]);
            ctx.builder.switch_to_block(loop_block);
            let idx = ctx.builder.use_var(idx_var);
            let done = ctx.builder.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual,
                idx,
                len_val,
            );
            ctx.builder
                .ins()
                .brif(done, break_block, &[], continue_block, &[]);

            ctx.builder.switch_to_block(continue_block);
            let offset = ctx.builder.ins().imul_imm(idx, 8);
            let data_ptr = ctx.builder.ins().iadd_imm(coll_val, 8);
            let elem_ptr = ctx.builder.ins().iadd(data_ptr, offset);
            let elem_val = ctx.builder.ins().load(I64, flags, elem_ptr, 0);
            let elem_val = ensure_f64_if(ctx, elem_val, elem_ty);

            let acc = ctx.builder.use_var(acc_var);
            let new_acc = call_callback(callback_val, &[acc, elem_val], &cb_params, &cb_ret, ctx)?;
            let new_acc = ensure_i64(ctx, new_acc);
            ctx.builder.def_var(acc_var, new_acc);
            let next_idx = ctx.builder.ins().iadd_imm(idx, 1);
            ctx.builder.def_var(idx_var, next_idx);
            ctx.builder.ins().jump(loop_block, &[]);

            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);
        }
        Ty::Range(_) => {
            let flags = MemFlagsData::new();
            let start_val = ctx.builder.ins().load(I64, flags, coll_val, 0);
            let current_var = ctx.new_var("__fold_current", I64);
            ctx.builder.def_var(current_var, start_val);

            ctx.builder.ins().jump(loop_block, &[]);
            ctx.builder.switch_to_block(loop_block);
            let current = ctx.builder.use_var(current_var);
            let done = super::range_iter::range_done(coll_val, current, ctx);
            ctx.builder
                .ins()
                .brif(done, break_block, &[], continue_block, &[]);

            ctx.builder.switch_to_block(continue_block);
            let elem_val = ensure_f64_if(ctx, current, elem_ty);

            let acc = ctx.builder.use_var(acc_var);
            let new_acc = call_callback(callback_val, &[acc, elem_val], &cb_params, &cb_ret, ctx)?;
            let new_acc = ensure_i64(ctx, new_acc);
            ctx.builder.def_var(acc_var, new_acc);
            let step_val = ctx.builder.ins().load(I64, flags, coll_val, 8);
            let next_raw = ctx.builder.ins().iadd(current, step_val);
            // SMI fix: (a<<1|1) + (b<<1|1) = (a+b)<<1 | 2. Subtrair 1 restaura tag.
            let next = ctx.builder.ins().iadd_imm(next_raw, -1);
            ctx.builder.def_var(current_var, next);
            ctx.builder.ins().jump(loop_block, &[]);

            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);
        }
        _ => {
            return Err(CodegenError::UnsupportedNode(format!(
                "Fold sobre tipo não-coleção: {coll_ty:?}"
            )));
        }
    }

    // break_block: retorna acc (convertendo I64→F64 se ret_ty é Float)
    ctx.builder.switch_to_block(break_block);
    ctx.builder.seal_block(break_block);
    let acc = ctx.builder.use_var(acc_var);
    Ok(ensure_f64_if(ctx, acc, ret_ty))
}

// ── List→Array conversão ─────────────────────────────────────

/// Converte uma List (Cons chain) em Array alocando um Array e copiando.
/// Percorre a lista, conta elementos, aloca array, copia.
pub(crate) fn list_to_array(
    list_val: cranelift_codegen::ir::Value,
    _elem_ty: &Ty,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    let arena = arena_handle(ctx);

    // Conta elementos: percorre a lista.
    let count_var = ctx.new_var("__l2a_count", I64);
    let zero = ctx.builder.ins().iconst(I64, 0);
    ctx.builder.def_var(count_var, zero);

    let current_var = ctx.new_var("__l2a_current", I64);
    ctx.builder.def_var(current_var, list_val);

    let count_loop = ctx.builder.create_block();
    let count_cont = ctx.builder.create_block();
    let count_done = ctx.builder.create_block();

    ctx.builder.ins().jump(count_loop, &[]);
    ctx.builder.switch_to_block(count_loop);
    let current = ctx.builder.use_var(current_var);
    let is_nil =
        ctx.builder
            .ins()
            .icmp_imm(cranelift_codegen::ir::condcodes::IntCC::Equal, current, 0);
    ctx.builder
        .ins()
        .brif(is_nil, count_done, &[], count_cont, &[]);

    ctx.builder.switch_to_block(count_cont);
    let flags = MemFlagsData::new();
    let tail = ctx.builder.ins().load(I64, flags, current, 8);
    let count = ctx.builder.use_var(count_var);
    let next_count = ctx.builder.ins().iadd_imm(count, 1);
    ctx.builder.def_var(count_var, next_count);
    ctx.builder.def_var(current_var, tail);
    ctx.builder.ins().jump(count_loop, &[]);

    ctx.builder.seal_block(count_loop);
    ctx.builder.seal_block(count_cont);

    ctx.builder.switch_to_block(count_done);
    ctx.builder.seal_block(count_done);
    let count = ctx.builder.use_var(count_var);

    // Aloca array: kata_rt_array_alloc(count, arena)
    let alloc_ref = ctx
        .ffi_refs
        .get("kata_rt_array_alloc")
        .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_array_alloc".into()))?;
    let alloc_call = ctx.builder.ins().call(*alloc_ref, &[count, arena]);
    let arr_ptr = ctx.builder.inst_results(alloc_call)[0];

    // Copia elementos: percorre a lista novamente, set cada elemento.
    let set_ref = ctx
        .ffi_refs
        .get("kata_rt_array_set")
        .ok_or_else(|| CodegenError::FfiSymbolNotFound("kata_rt_array_set".into()))?;

    let idx_var = ctx.new_var("__l2a_idx", I64);
    let zero = ctx.builder.ins().iconst(I64, 0);
    ctx.builder.def_var(idx_var, zero);
    ctx.builder.def_var(current_var, list_val);

    let copy_loop = ctx.builder.create_block();
    let copy_cont = ctx.builder.create_block();
    let copy_done = ctx.builder.create_block();

    ctx.builder.ins().jump(copy_loop, &[]);
    ctx.builder.switch_to_block(copy_loop);
    let current = ctx.builder.use_var(current_var);
    let is_nil =
        ctx.builder
            .ins()
            .icmp_imm(cranelift_codegen::ir::condcodes::IntCC::Equal, current, 0);
    ctx.builder
        .ins()
        .brif(is_nil, copy_done, &[], copy_cont, &[]);

    ctx.builder.switch_to_block(copy_cont);
    let flags = MemFlagsData::new();
    let head = ctx.builder.ins().load(I64, flags, current, 0);
    let tail = ctx.builder.ins().load(I64, flags, current, 8);
    let idx = ctx.builder.use_var(idx_var);
    let call = ctx.builder.ins().call(*set_ref, &[arr_ptr, idx, head]);
    let _ = ctx.builder.inst_results(call);
    let next_idx = ctx.builder.ins().iadd_imm(idx, 1);
    ctx.builder.def_var(idx_var, next_idx);
    ctx.builder.def_var(current_var, tail);
    ctx.builder.ins().jump(copy_loop, &[]);

    ctx.builder.seal_block(copy_loop);
    ctx.builder.seal_block(copy_cont);

    ctx.builder.switch_to_block(copy_done);
    ctx.builder.seal_block(copy_done);

    Ok(arr_ptr)
}
