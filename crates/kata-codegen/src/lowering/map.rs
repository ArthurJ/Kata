//! Map lowering — `lower_map` sobre List/Array/Range.
//!
//! Extraído de `collections_hof.rs`. Usa helpers compartilhados do parent
//! `collections_hof`: `arena_handle`, `call_callback`, `ensure_f64_if`,
//! `ensure_i64`, `extract_callback_sig`, `list_to_array`.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{InstBuilder, MemFlagsData};
use kata_core::ty::Ty;
use kata_inference::TypedExpr;

use super::CodegenError;
use super::LowerCtx;
use super::collections_hof::{
    arena_handle, call_callback, ensure_f64_if, ensure_i64, extract_callback_sig, list_to_array,
};

pub(crate) fn lower_map(
    callback: &kata_ast::Spanned<TypedExpr>,
    collection: &kata_ast::Spanned<TypedExpr>,
    coll_ty: &Ty,
    elem_ty: &Ty,
    _ret_ty: &Ty,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    let coll_val = super::expr::lower_expr(&collection.node, ctx)?;
    let callback_val = super::expr::lower_expr(&callback.node, ctx)?;

    let (cb_params, cb_ret) = extract_callback_sig(&callback.node);

    // ret_ty = List(B). O elemento da lista de resultado é B (cb_ret).
    let result_elem_ty = cb_ret.clone();

    // Arena para alocar Cons cells.
    let arena = arena_handle(ctx);

    // nil = kata_rt_list_nil()
    let nil_ref =
        ctx.ffi_refs
            .get("kata_rt_list_nil")
            .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                symbol: "kata_rt_list_nil".into(),
            })?;
    let nil_call = ctx.builder.ins().call(*nil_ref, &[]);
    let acc_var = ctx.new_var("__map_acc", I64);
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
            // current = coll_val; loop: if current == 0 break; head = load; tail = load
            let current_var = ctx.new_var("__map_current", I64);
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

            // Chama callback(head) → result
            let result = call_callback(callback_val, &[head_val], &cb_params, &cb_ret, ctx)?;
            let result_i64 = ensure_i64(ctx, result);

            // acc = cons(result, acc, arena)
            let acc = ctx.builder.use_var(acc_var);
            let call = ctx.builder.ins().call(*cons_ref, &[result_i64, acc, arena]);
            let new_acc = ctx.builder.inst_results(call)[0];
            ctx.builder.def_var(acc_var, new_acc);
            ctx.builder.def_var(current_var, tail_val);
            ctx.builder.ins().jump(loop_block, &[]);

            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);
        }
        Ty::Array(_) => {
            // len = load coll_val+0; idx = 0; loop: if idx >= len break
            let flags = MemFlagsData::new();
            let len_val = ctx.builder.ins().load(I64, flags, coll_val, 0);
            let idx_var = ctx.new_var("__map_idx", I64);
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

            let result = call_callback(callback_val, &[elem_val], &cb_params, &cb_ret, ctx)?;
            let result_i64 = ensure_i64(ctx, result);

            let acc = ctx.builder.use_var(acc_var);
            let call = ctx.builder.ins().call(*cons_ref, &[result_i64, acc, arena]);
            let new_acc = ctx.builder.inst_results(call)[0];
            ctx.builder.def_var(acc_var, new_acc);
            let next_idx = ctx.builder.ins().iadd_imm(idx, 1);
            ctx.builder.def_var(idx_var, next_idx);
            ctx.builder.ins().jump(loop_block, &[]);

            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);
        }
        Ty::Range(_) => {
            // start, step, end = load 0, 8, 16
            let flags = MemFlagsData::new();
            let start_val = ctx.builder.ins().load(I64, flags, coll_val, 0);
            let current_var = ctx.new_var("__map_current", I64);
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

            let result = call_callback(callback_val, &[elem_val], &cb_params, &cb_ret, ctx)?;
            let result_i64 = ensure_i64(ctx, result);

            let acc = ctx.builder.use_var(acc_var);
            let call = ctx.builder.ins().call(*cons_ref, &[result_i64, acc, arena]);
            let new_acc = ctx.builder.inst_results(call)[0];
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
            return Err(CodegenError::UnsupportedNode {
                node: format!("Map sobre tipo não-coleção: {coll_ty:?}"),
            });
        }
    }

    // break_block: acc contém a lista reversa.
    // Chama kata_rt_list_reverse(acc, arena) para restaurar a ordem.
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

    // Se coll_ty era Array, converter List→Array.
    if matches!(coll_ty, Ty::Array(_)) {
        list_to_array(reversed, &result_elem_ty, ctx)
    } else {
        Ok(reversed)
    }
}
