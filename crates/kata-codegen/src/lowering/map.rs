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
    limit: Option<&kata_ast::Spanned<TypedExpr>>,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    let limit_ctx = super::collections_hof::setup_limit(limit, ctx)?;
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

    // ── Tensor: percorre buffer flat, cria novo tensor com mesmo shape ──
    if let Ty::Tensor(_) = coll_ty {
        return lower_map_tensor(
            coll_val,
            callback_val,
            &cb_params,
            &cb_ret,
            &result_elem_ty,
            elem_ty,
            ctx,
        );
    }

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
            super::collections_hof::check_limit(&limit_ctx, break_block, ctx);
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
            super::collections_hof::increment_limit(&limit_ctx, ctx);
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
            super::collections_hof::check_limit(&limit_ctx, break_block, ctx);
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
            super::collections_hof::increment_limit(&limit_ctx, ctx);
            ctx.builder.ins().jump(loop_block, &[]);

            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);
        }
        Ty::Range(_) => {
            // Runtime guard: step == 0 → panic.
            super::range_iter::range_check_step(coll_val, elem_ty, ctx);
            // start, step, end = load 0, 8, 16
            let flags = MemFlagsData::new();
            let start_val = ctx.builder.ins().load(I64, flags, coll_val, 0);
            let current_var = ctx.new_var("__map_current", I64);
            ctx.builder.def_var(current_var, start_val);

            ctx.builder.ins().jump(loop_block, &[]);
            ctx.builder.switch_to_block(loop_block);
            let current = ctx.builder.use_var(current_var);
            let done = super::range_iter::range_done(coll_val, current, elem_ty, ctx);
            ctx.builder
                .ins()
                .brif(done, break_block, &[], continue_block, &[]);

            ctx.builder.switch_to_block(continue_block);
            super::collections_hof::check_limit(&limit_ctx, break_block, ctx);
            let elem_val = ensure_f64_if(ctx, current, elem_ty);

            let result = call_callback(callback_val, &[elem_val], &cb_params, &cb_ret, ctx)?;
            let result_i64 = ensure_i64(ctx, result);

            let acc = ctx.builder.use_var(acc_var);
            let call = ctx.builder.ins().call(*cons_ref, &[result_i64, acc, arena]);
            let new_acc = ctx.builder.inst_results(call)[0];
            ctx.builder.def_var(acc_var, new_acc);
            let next = super::range_iter::range_advance(coll_val, current, elem_ty, ctx);
            ctx.builder.def_var(current_var, next);
            super::collections_hof::increment_limit(&limit_ctx, ctx);
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

/// `map` sobre Tensor — percorre buffer flat, aplica callback, cria novo tensor.
///
/// Estratégia:
/// 1. Lê header do tensor de input (data_ptr, rank, shape_ptr, elem_type)
/// 2. Calcula n_elems = product(shape[0..rank])
/// 3. Aloca buffer de saída na arena
/// 4. Loop flat 0..n_elems: load elem do input, chama callback, store no output
/// 5. Aloca shape array, copia do input
/// 6. Chama kata_rt_tensor_new(data, rank, shape, elem_type)
fn lower_map_tensor(
    coll_val: cranelift_codegen::ir::Value,
    callback_val: cranelift_codegen::ir::Value,
    cb_params: &[Ty],
    cb_ret: &Ty,
    result_elem_ty: &Ty,
    elem_ty: &Ty,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    let flags = MemFlagsData::new();

    // Lê header do tensor de input
    // Offset 0: data_ptr (*mut u8)
    // Offset 8: rank (i64, SMI-tagged no runtime? Não — get_rank retorna i64 cru)
    // Offset 16: shape_ptr (*mut i64)
    // Offset 32: elem_type (i64)
    //
    // NOTA: O header armazena rank como i64 cru (não SMI-tagged).
    // elem_type também é cru (0 ou 1).
    let input_data_ptr = ctx.builder.ins().load(I64, flags, coll_val, 0);
    let rank_val = ctx.builder.ins().load(I64, flags, coll_val, 8);
    let shape_ptr = ctx.builder.ins().load(I64, flags, coll_val, 16);
    let _elem_type_val = ctx.builder.ins().load(I64, flags, coll_val, 32);

    // Calcula n_elems = product(shape[0..rank])
    // Loop compile-time: rank é conhecido em compile-time? Não — precisamos
    // de um loop runtime para multiplicar as dims.
    // Mas o rank é sempre ≤ 8 na prática. Podemos fazer um loop runtime.
    let n_elems = compute_n_elems(shape_ptr, rank_val, ctx)?;

    // Determina elem_type do resultado (pode mudar se callback muda tipo)
    let result_elem_type = match result_elem_ty {
        Ty::Prim(kata_core::ty::PrimTy::Float) => 1i64,
        _ => 0i64,
    };

    // Aloca buffer de saída: n_elems * 8 bytes
    let arena = arena_handle(ctx);
    let alloc_ref =
        ctx.ffi_refs
            .get("kata_rt_arena_alloc")
            .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                symbol: "kata_rt_arena_alloc".into(),
            })?;
    let rt_val = ctx.rt.unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
    let buf_size = ctx.builder.ins().imul_imm(n_elems, 8);
    let alloc_call = ctx
        .builder
        .ins()
        .call(*alloc_ref, &[rt_val, arena, buf_size]);
    let out_data_ptr = ctx.builder.inst_results(alloc_call)[0];

    // Loop: idx = 0; while idx < n_elems: load elem, call callback, store
    let loop_block = ctx.builder.create_block();
    let continue_block = ctx.builder.create_block();
    let break_block = ctx.builder.create_block();

    let idx_var = ctx.new_var("__map_t_idx", I64);
    let zero = ctx.builder.ins().iconst(I64, 0);
    ctx.builder.def_var(idx_var, zero);

    ctx.builder.ins().jump(loop_block, &[]);
    ctx.builder.switch_to_block(loop_block);
    let idx = ctx.builder.use_var(idx_var);
    let done = ctx.builder.ins().icmp(
        cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual,
        idx,
        n_elems,
    );
    ctx.builder
        .ins()
        .brif(done, break_block, &[], continue_block, &[]);

    ctx.builder.switch_to_block(continue_block);

    // Load elem do input: input_data_ptr + idx * 8
    let offset_in = ctx.builder.ins().imul_imm(idx, 8);
    let elem_addr = ctx.builder.ins().iadd(input_data_ptr, offset_in);
    let elem_val = ctx.builder.ins().load(I64, flags, elem_addr, 0);

    // Ensure F64 if elem_ty is Float (callback espera Float)
    let elem_val = ensure_f64_if(ctx, elem_val, elem_ty);

    // Chama callback(elem) → result
    let result = call_callback(callback_val, &[elem_val], cb_params, cb_ret, ctx)?;
    let result_i64 = ensure_i64(ctx, result);

    // Store no output: out_data_ptr + idx * 8
    let out_addr = ctx.builder.ins().iadd(out_data_ptr, offset_in);
    ctx.builder.ins().store(flags, result_i64, out_addr, 0);

    // idx++
    let next_idx = ctx.builder.ins().iadd_imm(idx, 1);
    ctx.builder.def_var(idx_var, next_idx);
    ctx.builder.ins().jump(loop_block, &[]);

    ctx.builder.seal_block(loop_block);
    ctx.builder.seal_block(continue_block);

    // break_block: cria novo tensor com mesmo shape
    ctx.builder.switch_to_block(break_block);
    ctx.builder.seal_block(break_block);

    // Aloca shape array de saída: rank * 8 bytes (copia do input)
    let shape_size = ctx.builder.ins().imul_imm(rank_val, 8);
    let shape_alloc = ctx
        .builder
        .ins()
        .call(*alloc_ref, &[rt_val, arena, shape_size]);
    let out_shape_ptr = ctx.builder.inst_results(shape_alloc)[0];

    // Copia shape do input para output
    let copy_loop = ctx.builder.create_block();
    let copy_continue = ctx.builder.create_block();
    let copy_done = ctx.builder.create_block();

    let copy_idx_var = ctx.new_var("__map_t_copy_idx", I64);
    ctx.builder.def_var(copy_idx_var, zero);
    ctx.builder.ins().jump(copy_loop, &[]);
    ctx.builder.switch_to_block(copy_loop);
    let copy_idx = ctx.builder.use_var(copy_idx_var);
    let copy_done_cond = ctx.builder.ins().icmp(
        cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual,
        copy_idx,
        rank_val,
    );
    ctx.builder
        .ins()
        .brif(copy_done_cond, copy_done, &[], copy_continue, &[]);

    ctx.builder.switch_to_block(copy_continue);
    let copy_offset = ctx.builder.ins().imul_imm(copy_idx, 8);
    let src_shape_addr = ctx.builder.ins().iadd(shape_ptr, copy_offset);
    let dim_val = ctx.builder.ins().load(I64, flags, src_shape_addr, 0);
    let dst_shape_addr = ctx.builder.ins().iadd(out_shape_ptr, copy_offset);
    ctx.builder.ins().store(flags, dim_val, dst_shape_addr, 0);
    let next_copy_idx = ctx.builder.ins().iadd_imm(copy_idx, 1);
    ctx.builder.def_var(copy_idx_var, next_copy_idx);
    ctx.builder.ins().jump(copy_loop, &[]);

    ctx.builder.seal_block(copy_loop);
    ctx.builder.seal_block(copy_continue);
    ctx.builder.switch_to_block(copy_done);
    ctx.builder.seal_block(copy_done);

    // Chama kata_rt_tensor_new(data, rank, shape, elem_type)
    let tensor_new_ref =
        ctx.ffi_refs
            .get("kata_rt_tensor_new")
            .ok_or_else(|| CodegenError::FfiSymbolNotFound {
                symbol: "kata_rt_tensor_new".into(),
            })?;
    let result_elem_type_val = ctx.builder.ins().iconst(I64, result_elem_type);
    let tensor_call = ctx.builder.ins().call(
        *tensor_new_ref,
        &[out_data_ptr, rank_val, out_shape_ptr, result_elem_type_val],
    );
    let tensor_ptr = ctx.builder.inst_results(tensor_call)[0];
    Ok(tensor_ptr)
}

/// Calcula n_elems = product(shape[0..rank]) em runtime.
/// Loop: i = 0; acc = 1; while i < rank: acc *= shape[i]; i++
fn compute_n_elems(
    shape_ptr: cranelift_codegen::ir::Value,
    rank_val: cranelift_codegen::ir::Value,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, CodegenError> {
    let flags = MemFlagsData::new();

    let loop_block = ctx.builder.create_block();
    let continue_block = ctx.builder.create_block();
    let done_block = ctx.builder.create_block();

    let i_var = ctx.new_var("__map_t_ne_i", I64);
    let acc_var = ctx.new_var("__map_t_ne_acc", I64);
    let zero = ctx.builder.ins().iconst(I64, 0);
    let one = ctx.builder.ins().iconst(I64, 1);
    ctx.builder.def_var(i_var, zero);
    ctx.builder.def_var(acc_var, one);

    ctx.builder.ins().jump(loop_block, &[]);
    ctx.builder.switch_to_block(loop_block);
    let i = ctx.builder.use_var(i_var);
    let done = ctx.builder.ins().icmp(
        cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual,
        i,
        rank_val,
    );
    ctx.builder
        .ins()
        .brif(done, done_block, &[], continue_block, &[]);

    ctx.builder.switch_to_block(continue_block);
    let offset = ctx.builder.ins().imul_imm(i, 8);
    let dim_addr = ctx.builder.ins().iadd(shape_ptr, offset);
    let dim = ctx.builder.ins().load(I64, flags, dim_addr, 0);
    let acc = ctx.builder.use_var(acc_var);
    let new_acc = ctx.builder.ins().imul(acc, dim);
    ctx.builder.def_var(acc_var, new_acc);
    let next_i = ctx.builder.ins().iadd_imm(i, 1);
    ctx.builder.def_var(i_var, next_i);
    ctx.builder.ins().jump(loop_block, &[]);

    ctx.builder.seal_block(loop_block);
    ctx.builder.seal_block(continue_block);
    ctx.builder.switch_to_block(done_block);
    ctx.builder.seal_block(done_block);

    let acc = ctx.builder.use_var(acc_var);
    Ok(acc)
}
