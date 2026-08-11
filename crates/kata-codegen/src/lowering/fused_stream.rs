//! FusedStream lowering — cadeia fused de map/filter em uma única passagem.
//!
//! Extraído de `collections_hof.rs`: `lower_fused_stream` + `apply_stages`.
//! Usa helpers compartilhados (`ensure_i64`, `ensure_f64_if`, `arena_handle`,
//! `call_callback`, `extract_callback_sig`, `list_to_array`) do parent
//! `collections_hof`.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{InstBuilder, MemFlagsData};
use kata_core::ty::Ty;
use kata_inference::{FusedStage, TypedExpr};

use super::CodegenError;
use super::LowerCtx;
use super::collections_hof::{
    arena_handle, call_callback, ensure_f64_if, ensure_i64, extract_callback_sig, list_to_array,
};

pub(crate) fn lower_fused_stream(
    stages: &[FusedStage],
    source: &kata_ast::Spanned<TypedExpr>,
    coll_ty: &Ty,
    source_elem_ty: &Ty,
    result_elem_ty: &Ty,
    _ret_ty: &Ty,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    let coll_val = super::expr::lower_expr(&source.node, ctx)?;

    // Lowera todos os callbacks dos stages e extrai sig info.
    let mut stage_callbacks: Vec<cranelift_codegen::ir::Value> = Vec::new();
    let mut stage_params: Vec<Vec<Ty>> = Vec::new();
    let mut stage_rets: Vec<Ty> = Vec::new();
    for stage in stages {
        let cb_expr = match stage {
            FusedStage::Filter { callback, .. } | FusedStage::Map { callback, .. } => callback,
        };
        let cb_val = super::expr::lower_expr(&cb_expr.node, ctx)?;
        let (cb_params, cb_ret) = extract_callback_sig(&cb_expr.node);
        stage_callbacks.push(cb_val);
        stage_params.push(cb_params);
        stage_rets.push(cb_ret);
    }

    let arena = arena_handle(ctx);

    // nil = kata_rt_list_nil()
    let nil_ref =
        ctx.ffi_refs
            .get("kata_rt_list_nil")
            .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                symbol: "kata_rt_list_nil".into(),
            })?;
    let nil_call = ctx.builder.ins().call(*nil_ref, &[]);
    let acc_var = ctx.new_var("__fused_acc", I64);
    ctx.builder
        .def_var(acc_var, ctx.builder.inst_results(nil_call)[0]);

    let cons_ref =
        ctx.ffi_refs
            .get("kata_rt_list_cons")
            .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                symbol: "kata_rt_list_cons".into(),
            })?;

    let loop_block = ctx.builder.create_block();
    let continue_block = ctx.builder.create_block();
    let break_block = ctx.builder.create_block();

    match coll_ty {
        Ty::List(_) => {
            let current_var = ctx.new_var("__fused_current", I64);
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
            let head_val = ensure_f64_if(ctx, head_val, source_elem_ty);
            let tail_val = ctx.builder.ins().load(I64, flags, current, 8);

            let (result_val, keep) = apply_stages(
                stages,
                head_val,
                &stage_callbacks,
                &stage_params,
                &stage_rets,
                ctx,
            )?;

            // Se keep != 0, faz cons; senao, skip.
            let should_cons = ctx.builder.ins().icmp_imm(
                cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                keep,
                0,
            );
            let cons_block = ctx.builder.create_block();
            let skip_block = ctx.builder.create_block();
            let after_cons = ctx.builder.create_block();
            ctx.builder
                .ins()
                .brif(should_cons, cons_block, &[], skip_block, &[]);

            // cons_block: faz cons(result_val, acc, arena).
            ctx.builder.switch_to_block(cons_block);
            ctx.builder.seal_block(cons_block);
            let val_i64 = ensure_i64(ctx, result_val);
            let acc = ctx.builder.use_var(acc_var);
            let call = ctx.builder.ins().call(*cons_ref, &[val_i64, acc, arena]);
            let new_acc = ctx.builder.inst_results(call)[0];
            ctx.builder.def_var(acc_var, new_acc);
            ctx.builder.ins().jump(after_cons, &[]);

            // skip_block: nao faz cons.
            ctx.builder.switch_to_block(skip_block);
            ctx.builder.seal_block(skip_block);
            ctx.builder.ins().jump(after_cons, &[]);

            ctx.builder.switch_to_block(after_cons);
            ctx.builder.seal_block(after_cons);

            ctx.builder.def_var(current_var, tail_val);
            ctx.builder.ins().jump(loop_block, &[]);

            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);
        }
        Ty::Array(_) => {
            let flags = MemFlagsData::new();
            let len_val = ctx.builder.ins().load(I64, flags, coll_val, 0);
            let idx_var = ctx.new_var("__fused_idx", I64);
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
            let elem_val = ensure_f64_if(ctx, elem_val, source_elem_ty);

            let (result_val, keep) = apply_stages(
                stages,
                elem_val,
                &stage_callbacks,
                &stage_params,
                &stage_rets,
                ctx,
            )?;

            let should_cons = ctx.builder.ins().icmp_imm(
                cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                keep,
                0,
            );
            let cons_block = ctx.builder.create_block();
            let skip_block = ctx.builder.create_block();
            let after_cons = ctx.builder.create_block();
            ctx.builder
                .ins()
                .brif(should_cons, cons_block, &[], skip_block, &[]);

            ctx.builder.switch_to_block(cons_block);
            ctx.builder.seal_block(cons_block);
            let val_i64 = ensure_i64(ctx, result_val);
            let acc = ctx.builder.use_var(acc_var);
            let call = ctx.builder.ins().call(*cons_ref, &[val_i64, acc, arena]);
            let new_acc = ctx.builder.inst_results(call)[0];
            ctx.builder.def_var(acc_var, new_acc);
            ctx.builder.ins().jump(after_cons, &[]);

            ctx.builder.switch_to_block(skip_block);
            ctx.builder.seal_block(skip_block);
            ctx.builder.ins().jump(after_cons, &[]);

            ctx.builder.switch_to_block(after_cons);
            ctx.builder.seal_block(after_cons);

            let next_idx = ctx.builder.ins().iadd_imm(idx, 1);
            ctx.builder.def_var(idx_var, next_idx);
            ctx.builder.ins().jump(loop_block, &[]);

            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);
        }
        Ty::Range(_) => {
            let flags = MemFlagsData::new();
            let start_val = ctx.builder.ins().load(I64, flags, coll_val, 0);
            let current_var = ctx.new_var("__fused_current", I64);
            ctx.builder.def_var(current_var, start_val);

            ctx.builder.ins().jump(loop_block, &[]);
            ctx.builder.switch_to_block(loop_block);
            let current = ctx.builder.use_var(current_var);
            let done = super::range_iter::range_done(coll_val, current, ctx);
            ctx.builder
                .ins()
                .brif(done, break_block, &[], continue_block, &[]);

            ctx.builder.switch_to_block(continue_block);
            let elem_val = ensure_f64_if(ctx, current, source_elem_ty);

            let (result_val, keep) = apply_stages(
                stages,
                elem_val,
                &stage_callbacks,
                &stage_params,
                &stage_rets,
                ctx,
            )?;

            let should_cons = ctx.builder.ins().icmp_imm(
                cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                keep,
                0,
            );
            let cons_block = ctx.builder.create_block();
            let skip_block = ctx.builder.create_block();
            let after_cons = ctx.builder.create_block();
            ctx.builder
                .ins()
                .brif(should_cons, cons_block, &[], skip_block, &[]);

            ctx.builder.switch_to_block(cons_block);
            ctx.builder.seal_block(cons_block);
            let val_i64 = ensure_i64(ctx, result_val);
            let acc = ctx.builder.use_var(acc_var);
            let call = ctx.builder.ins().call(*cons_ref, &[val_i64, acc, arena]);
            let new_acc = ctx.builder.inst_results(call)[0];
            ctx.builder.def_var(acc_var, new_acc);
            ctx.builder.ins().jump(after_cons, &[]);

            ctx.builder.switch_to_block(skip_block);
            ctx.builder.seal_block(skip_block);
            ctx.builder.ins().jump(after_cons, &[]);

            ctx.builder.switch_to_block(after_cons);
            ctx.builder.seal_block(after_cons);

            let step_val = ctx.builder.ins().load(I64, flags, coll_val, 8);
            let next_raw = ctx.builder.ins().iadd(current, step_val);
            let next = ctx.builder.ins().iadd_imm(next_raw, -1);
            ctx.builder.def_var(current_var, next);
            ctx.builder.ins().jump(loop_block, &[]);

            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);
        }
        _ => {
            return Err(CodegenError::UnsupportedNode {
                node: format!("FusedStream sobre tipo nao-colecao: {coll_ty:?}"),
            });
        }
    }

    // break_block: acc contem a lista reversa.
    ctx.builder.switch_to_block(break_block);
    ctx.builder.seal_block(break_block);
    let acc = ctx.builder.use_var(acc_var);
    let reverse_ref = ctx.ffi_refs.get("kata_rt_list_reverse").ok_or_else(|| {
        CodegenError::FfiSymbolNotFound {
            symbol: "kata_rt_list_reverse".into(),
        }
    })?;
    let call = ctx.builder.ins().call(*reverse_ref, &[acc, arena]);
    let reversed = ctx.builder.inst_results(call)[0];

    if matches!(coll_ty, Ty::Array(_)) {
        list_to_array(reversed, result_elem_ty, ctx)
    } else {
        Ok(reversed)
    }
}

/// Aplica os estagios da cadeia em sequencia a um valor.
/// Retorna (val, keep_flag) onde keep_flag é I64 (0 = descartado, 1 = keep).
/// Cada Filter faz AND logico com o keep_flag atual: se qualquer
/// Filter retorna false, keep_flag vira 0 e o caller nao faz cons.
fn apply_stages(
    stages: &[FusedStage],
    mut val: cranelift_codegen::ir::Value,
    stage_callbacks: &[cranelift_codegen::ir::Value],
    stage_params: &[Vec<Ty>],
    stage_rets: &[Ty],
    ctx: &mut LowerCtx,
) -> Result<(cranelift_codegen::ir::Value, cranelift_codegen::ir::Value), CodegenError> {
    // keep_flag comeca em 1 (keep).
    let mut keep = ctx.builder.ins().iconst(I64, 1);

    for (i, stage) in stages.iter().enumerate() {
        match stage {
            FusedStage::Filter { .. } => {
                // Se keep já é 0, skip o predicado (AND curto-circuito).
                // Mas Cranelift nao tem short-circuit nativo em SSA.
                // Solucao: sempre chama o predicado, faz AND com keep.
                let pred_result = call_callback(
                    stage_callbacks[i],
                    &[val],
                    &stage_params[i],
                    &stage_rets[i],
                    ctx,
                )?;
                // Boolean: 0 = false, 1 = true (cru, sem SMI tag).
                // keep = keep AND pred_result (ambos 0/1 cru).
                let pred_i64 = ensure_i64(ctx, pred_result);
                keep = ctx.builder.ins().band(keep, pred_i64);
            }
            FusedStage::Map { .. } => {
                // Se keep é 0, o Map não precisa ser aplicado (elemento ja descartado).
                // Mas em SSA não dá para skip condicional sem blocks.
                // Aplicamos sempre — o resultado é ignorado se keep = 0.
                let result = call_callback(
                    stage_callbacks[i],
                    &[val],
                    &stage_params[i],
                    &stage_rets[i],
                    ctx,
                )?;
                val = ensure_i64(ctx, result);
            }
        }
    }
    Ok((val, keep))
}
