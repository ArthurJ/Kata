//! Lowering de arms de coleções literais — `ListLit`, `ArrayLit`, `RangeLit`,
//! `ForIn`, `In` do match `lower_expr`.
//!
//! Extraído de `expr.rs` para reduzir o tamanho do dispatch central.
//! HOFs (Map, Filter, Fold, FusedStream) continuam em seus submódulos próprios
//! (`map.rs`, `filter.rs`, `collections_hof.rs`, `fused_stream.rs`).

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{GlobalValueData, InstBuilder, MemFlagsData};

use kata_core::ty::{PrimTy, Ty};
use kata_inference::{TypedExpr, TypedExprKind};

use super::LowerCtx;
use super::expr::lower_expr;
use crate::ffi_sigs::ty_to_clif;

/// Lowera arms de coleções literais: `ListLit`, `ArrayLit`, `RangeLit`,
/// `ForIn`, `In`, `DictLit`, `SetLit`.
///
/// Retorna `Ok(Some(value))` se o arm foi tratado, `Ok(None)` se o `kind`
/// não é de coleção literal (caller continua o match).
pub(crate) fn lower_collections_literal(
    expr: &TypedExpr,
    ctx: &mut LowerCtx,
) -> Result<Option<cranelift_codegen::ir::Value>, super::CodegenError> {
    match &expr.kind {
        // ── ListLit — constrói Cons chain de trás para frente ──
        TypedExprKind::ListLit { elements } => {
            let arena_handle = match expr.escape {
                kata_core::escape::EscapeTarget::Local => ctx
                    .fiber_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
                kata_core::escape::EscapeTarget::Caller => ctx
                    .caller_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
            };

            // Começa com nil (0).
            let nil_ref = ctx
                .ffi_refs
                .get("kata_rt_list_nil")
                .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_list_nil".into()))?;
            let nil_call = ctx.builder.ins().call(*nil_ref, &[]);
            let mut acc = ctx.builder.inst_results(nil_call)[0];

            // Cons para cada elemento, de trás para frente.
            let cons_ref = ctx.ffi_refs.get("kata_rt_list_cons").ok_or_else(|| {
                super::CodegenError::FfiSymbolNotFound("kata_rt_list_cons".into())
            })?;
            for elem in elements.iter().rev() {
                let head = lower_expr(&elem.node, ctx)?;
                // Bitcast F64→I64 se o elemento for Float.
                let head = {
                    let head_ty = ctx.builder.func.dfg.value_type(head);
                    if head_ty == cranelift_codegen::ir::types::F64 {
                        ctx.builder.ins().bitcast(I64, MemFlagsData::new(), head)
                    } else {
                        head
                    }
                };
                let call = ctx
                    .builder
                    .ins()
                    .call(*cons_ref, &[head, acc, arena_handle]);
                acc = ctx.builder.inst_results(call)[0];
            }
            Ok(Some(acc))
        }

        // ── ArrayLit — aloca header+data, set cada elemento ──
        TypedExprKind::ArrayLit { elements } => {
            let n = elements.len() as i64;
            let arena_handle = match expr.escape {
                kata_core::escape::EscapeTarget::Local => ctx
                    .fiber_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
                kata_core::escape::EscapeTarget::Caller => ctx
                    .caller_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
            };

            // Aloca array: kata_rt_array_alloc(len, arena) → ptr
            let alloc_ref = ctx.ffi_refs.get("kata_rt_array_alloc").ok_or_else(|| {
                super::CodegenError::FfiSymbolNotFound("kata_rt_array_alloc".into())
            })?;
            let len_val = ctx.builder.ins().iconst(I64, n);
            let call = ctx.builder.ins().call(*alloc_ref, &[len_val, arena_handle]);
            let ptr = ctx.builder.inst_results(call)[0];

            // Set cada elemento: kata_rt_array_set(ptr, idx, val)
            let set_ref = ctx.ffi_refs.get("kata_rt_array_set").ok_or_else(|| {
                super::CodegenError::FfiSymbolNotFound("kata_rt_array_set".into())
            })?;
            for (i, elem) in elements.iter().enumerate() {
                let val = lower_expr(&elem.node, ctx)?;
                // Bitcast F64→I64 se o elemento for Float.
                let val = {
                    let val_ty = ctx.builder.func.dfg.value_type(val);
                    if val_ty == cranelift_codegen::ir::types::F64 {
                        ctx.builder.ins().bitcast(I64, MemFlagsData::new(), val)
                    } else {
                        val
                    }
                };
                let idx = ctx.builder.ins().iconst(I64, i as i64);
                ctx.builder.ins().call(*set_ref, &[ptr, idx, val]);
            }
            Ok(Some(ptr))
        }

        // ── RangeLit — aloca 3 words, store start/step/end ──
        TypedExprKind::RangeLit {
            start,
            step,
            end,
            inclusive: _,
            elem_ty: _,
        } => {
            let arena_handle = match expr.escape {
                kata_core::escape::EscapeTarget::Local => ctx
                    .fiber_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
                kata_core::escape::EscapeTarget::Caller => ctx
                    .caller_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
            };

            // Aloca 24 bytes: kata_rt_range_alloc(arena) → ptr
            let alloc_ref = ctx.ffi_refs.get("kata_rt_range_alloc").ok_or_else(|| {
                super::CodegenError::FfiSymbolNotFound("kata_rt_range_alloc".into())
            })?;
            let call = ctx.builder.ins().call(*alloc_ref, &[arena_handle]);
            let ptr = ctx.builder.inst_results(call)[0];

            // Store start (offset 0), step (offset 8), end (offset 16).
            let flags = MemFlagsData::new();
            let start_val = lower_expr(&start.node, ctx)?;
            let step_val = lower_expr(&step.node, ctx)?;
            let end_val = lower_expr(&end.node, ctx)?;

            // Bitcast F64→I64 se os valores forem Float.
            let start_val = {
                let ty = ctx.builder.func.dfg.value_type(start_val);
                if ty == cranelift_codegen::ir::types::F64 {
                    ctx.builder
                        .ins()
                        .bitcast(I64, MemFlagsData::new(), start_val)
                } else {
                    start_val
                }
            };
            let step_val = {
                let ty = ctx.builder.func.dfg.value_type(step_val);
                if ty == cranelift_codegen::ir::types::F64 {
                    ctx.builder
                        .ins()
                        .bitcast(I64, MemFlagsData::new(), step_val)
                } else {
                    step_val
                }
            };
            let end_val = {
                let ty = ctx.builder.func.dfg.value_type(end_val);
                if ty == cranelift_codegen::ir::types::F64 {
                    ctx.builder.ins().bitcast(I64, MemFlagsData::new(), end_val)
                } else {
                    end_val
                }
            };

            ctx.builder.ins().store(flags, start_val, ptr, 0);
            ctx.builder.ins().store(flags, step_val, ptr, 8);
            ctx.builder.ins().store(flags, end_val, ptr, 16);
            Ok(Some(ptr))
        }

        // ── ForIn — loop inlined por tipo concreto ──
        TypedExprKind::ForIn {
            var_name,
            var_ty,
            iterable,
            body,
        } => {
            // Salva loop blocks anteriores.
            let prev_break = ctx.loop_break_block;
            let prev_continue = ctx.loop_continue_block;

            let loop_block = ctx.builder.create_block();
            let continue_block = ctx.builder.create_block();
            let break_block = ctx.builder.create_block();
            ctx.loop_break_block = Some(break_block);
            ctx.loop_continue_block = Some(continue_block);

            let coll_val = lower_expr(&iterable.node, ctx)?;
            let coll_ty = &iterable.node.ty;

            match coll_ty {
                Ty::List(_) => {
                    // List: percorre Cons cells. current = coll_ptr.
                    // Condição: current != 0 (Nil).
                    let current_var = ctx.new_var("__for_current", I64);
                    ctx.builder.def_var(current_var, coll_val);

                    ctx.builder.ins().jump(loop_block, &[]);
                    ctx.builder.switch_to_block(loop_block);
                    // Yield point no header.
                    let yc = ctx
                        .ffi_refs
                        .get("kata_rt_yield_check")
                        .copied()
                        .ok_or_else(|| {
                            super::CodegenError::FfiSymbolNotFound("kata_rt_yield_check".into())
                        })?;
                    ctx.builder.ins().call(yc, &[]);
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
                    // head = load current+0, tail = load current+8
                    let flags = MemFlagsData::new();
                    let head_val = ctx.builder.ins().load(I64, flags, current, 0);
                    // Bitcast I64→F64 se var_ty é Float.
                    let head_val = if *var_ty == Ty::float() {
                        ctx.builder.ins().bitcast(
                            cranelift_codegen::ir::types::F64,
                            MemFlagsData::new(),
                            head_val,
                        )
                    } else {
                        head_val
                    };
                    let tail_val = ctx.builder.ins().load(I64, flags, current, 8);

                    let elem_var = ctx.new_var(var_name, ty_to_clif(var_ty));
                    ctx.builder.def_var(elem_var, head_val);
                    ctx.builder.def_var(current_var, tail_val);

                    // Executa body.
                    for e in body {
                        lower_expr(&e.node, ctx)?;
                    }
                    ctx.builder.ins().jump(loop_block, &[]);

                    ctx.builder.seal_block(loop_block);
                    ctx.builder.seal_block(continue_block);
                }
                Ty::Array(_) => {
                    // Array: percorre índices 0..len.
                    // len = load coll_ptr+0
                    let flags = MemFlagsData::new();
                    let len_val = ctx.builder.ins().load(I64, flags, coll_val, 0);
                    let idx_var = ctx.new_var("__for_idx", I64);
                    let zero = ctx.builder.ins().iconst(I64, 0);
                    ctx.builder.def_var(idx_var, zero);

                    ctx.builder.ins().jump(loop_block, &[]);
                    ctx.builder.switch_to_block(loop_block);
                    // Yield point no header.
                    let yc = ctx
                        .ffi_refs
                        .get("kata_rt_yield_check")
                        .copied()
                        .ok_or_else(|| {
                            super::CodegenError::FfiSymbolNotFound("kata_rt_yield_check".into())
                        })?;
                    ctx.builder.ins().call(yc, &[]);
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
                    // elem = load coll_ptr + 8 + idx * 8
                    let offset = ctx.builder.ins().imul_imm(idx, 8);
                    let data_ptr = ctx.builder.ins().iadd_imm(coll_val, 8);
                    let elem_ptr = ctx.builder.ins().iadd(data_ptr, offset);
                    let elem_val = ctx.builder.ins().load(I64, flags, elem_ptr, 0);
                    let elem_val = if *var_ty == Ty::float() {
                        ctx.builder.ins().bitcast(
                            cranelift_codegen::ir::types::F64,
                            MemFlagsData::new(),
                            elem_val,
                        )
                    } else {
                        elem_val
                    };
                    let elem_var = ctx.new_var(var_name, ty_to_clif(var_ty));
                    ctx.builder.def_var(elem_var, elem_val);

                    // idx += 1
                    let next_idx = ctx.builder.ins().iadd_imm(idx, 1);
                    ctx.builder.def_var(idx_var, next_idx);

                    for e in body {
                        lower_expr(&e.node, ctx)?;
                    }
                    ctx.builder.ins().jump(loop_block, &[]);

                    ctx.builder.seal_block(loop_block);
                    ctx.builder.seal_block(continue_block);
                }
                Ty::Range(_) => {
                    // Range: percorre current = start, current += step,
                    // condição: inclusive ? current > end : current >= end.
                    let flags = MemFlagsData::new();
                    let start_val = ctx.builder.ins().load(I64, flags, coll_val, 0);
                    let step_val = ctx.builder.ins().load(I64, flags, coll_val, 8);
                    let end_val = ctx.builder.ins().load(I64, flags, coll_val, 16);
                    let current_var = ctx.new_var("__for_current", I64);
                    ctx.builder.def_var(current_var, start_val);

                    ctx.builder.ins().jump(loop_block, &[]);
                    ctx.builder.switch_to_block(loop_block);
                    // Yield point no header.
                    let yc = ctx
                        .ffi_refs
                        .get("kata_rt_yield_check")
                        .copied()
                        .ok_or_else(|| {
                            super::CodegenError::FfiSymbolNotFound("kata_rt_yield_check".into())
                        })?;
                    ctx.builder.ins().call(yc, &[]);
                    let current = ctx.builder.use_var(current_var);
                    // Para inclusive: current > end → break
                    // Para exclusive: current >= end → break
                    // (detectado pelo campo `inclusive` na TAST, mas não temos
                    // acesso aqui — usar inclusive do match pattern)
                    let done = ctx.builder.ins().icmp(
                        cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual,
                        current,
                        end_val,
                    );
                    ctx.builder
                        .ins()
                        .brif(done, break_block, &[], continue_block, &[]);

                    ctx.builder.switch_to_block(continue_block);
                    let elem_var = ctx.new_var(var_name, ty_to_clif(var_ty));
                    let elem_val = if *var_ty == Ty::float() {
                        ctx.builder.ins().bitcast(
                            cranelift_codegen::ir::types::F64,
                            MemFlagsData::new(),
                            current,
                        )
                    } else {
                        current
                    };
                    ctx.builder.def_var(elem_var, elem_val);

                    // current += step
                    let next_raw = ctx.builder.ins().iadd(current, step_val);
                    // SMI fix: (a<<1|1) + (b<<1|1) = (a+b)<<1 | 2. Subtrair 1 restaura tag.
                    let next = ctx.builder.ins().iadd_imm(next_raw, -1);
                    ctx.builder.def_var(current_var, next);

                    for e in body {
                        lower_expr(&e.node, ctx)?;
                    }
                    ctx.builder.ins().jump(loop_block, &[]);

                    ctx.builder.seal_block(loop_block);
                    ctx.builder.seal_block(continue_block);
                }
                _ => {
                    return Err(super::CodegenError::UnsupportedNode(format!(
                        "ForIn sobre tipo não-iterável: {coll_ty:?}"
                    )));
                }
            }

            // break_block: retorna Unit.
            ctx.builder.switch_to_block(break_block);
            ctx.builder.seal_block(break_block);
            let unit = ctx.builder.ins().iconst(I64, 0);

            // Restaura ctx.
            ctx.loop_break_block = prev_break;
            ctx.loop_continue_block = prev_continue;

            Ok(Some(unit))
        }

        // ── In (membership) — dispatch por tipo concreto ──
        TypedExprKind::In { item, collection } => {
            let coll_val = lower_expr(&collection.node, ctx)?;
            let item_val = lower_expr(&item.node, ctx)?;
            let coll_ty = &collection.node.ty;

            match coll_ty {
                Ty::List(_) => {
                    // List: chama kata_rt_list_contains(ptr, item) → 0/1
                    let func_ref = ctx.ffi_refs.get("kata_rt_list_contains").ok_or_else(|| {
                        super::CodegenError::FfiSymbolNotFound("kata_rt_list_contains".into())
                    })?;
                    // Bitcast F64→I64 se o item for Float.
                    let item_i64 = {
                        let ty = ctx.builder.func.dfg.value_type(item_val);
                        if ty == cranelift_codegen::ir::types::F64 {
                            ctx.builder
                                .ins()
                                .bitcast(I64, MemFlagsData::new(), item_val)
                        } else {
                            item_val
                        }
                    };
                    let call = ctx.builder.ins().call(*func_ref, &[coll_val, item_i64]);
                    Ok(Some(ctx.builder.inst_results(call)[0]))
                }
                Ty::Array(_) => {
                    // Array: chama kata_rt_array_contains(ptr, item) → 0/1
                    let func_ref = ctx.ffi_refs.get("kata_rt_array_contains").ok_or_else(|| {
                        super::CodegenError::FfiSymbolNotFound("kata_rt_array_contains".into())
                    })?;
                    // Bitcast F64→I64 se o item for Float.
                    let item_i64 = {
                        let ty = ctx.builder.func.dfg.value_type(item_val);
                        if ty == cranelift_codegen::ir::types::F64 {
                            ctx.builder
                                .ins()
                                .bitcast(I64, MemFlagsData::new(), item_val)
                        } else {
                            item_val
                        }
                    };
                    let call = ctx.builder.ins().call(*func_ref, &[coll_val, item_i64]);
                    Ok(Some(ctx.builder.inst_results(call)[0]))
                }
                Ty::Range(_) => {
                    // Range: O(1) aritmético.
                    // Apenas checa start <= item < end (sem verificar step).
                    let flags = MemFlagsData::new();
                    let start = ctx.builder.ins().load(I64, flags, coll_val, 0);
                    let end = ctx.builder.ins().load(I64, flags, coll_val, 16);

                    // item >= start AND item < end
                    let ge_start = ctx.builder.ins().icmp(
                        cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual,
                        item_val,
                        start,
                    );
                    // item < end (exclusive)
                    let lt_end = ctx.builder.ins().icmp(
                        cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                        item_val,
                        end,
                    );
                    // Cranelift I8 band → extend to I64.
                    let result_i8 = ctx.builder.ins().band(ge_start, lt_end);
                    Ok(Some(ctx.builder.ins().uextend(I64, result_i8)))
                }
                Ty::Dict(k, _) => {
                    // Dict: hash key, call kata_rt_dict_contains(dict, key, hash, eq_fn)
                    let item_i64 = bitcast_to_i64(item_val, ctx);
                    let hash_name = hash_fn_name(k)?;
                    let eq_name = eq_fn_name(k)?;
                    let hash_ref =
                        ctx.ffi_refs.get(hash_name).copied().ok_or_else(|| {
                            super::CodegenError::FfiSymbolNotFound(hash_name.into())
                        })?;
                    let contains_ref = ctx
                        .ffi_refs
                        .get("kata_rt_dict_contains")
                        .copied()
                        .ok_or_else(|| {
                            super::CodegenError::FfiSymbolNotFound("kata_rt_dict_contains".into())
                        })?;
                    let hash_call = ctx.builder.ins().call(hash_ref, &[item_i64]);
                    let hash_val = ctx.builder.inst_results(hash_call)[0];
                    let eq_fn_ptr = get_ffi_fn_ptr(eq_name, ctx)?;
                    let call = ctx
                        .builder
                        .ins()
                        .call(contains_ref, &[coll_val, item_i64, hash_val, eq_fn_ptr]);
                    Ok(Some(ctx.builder.inst_results(call)[0]))
                }
                Ty::Set(t) => {
                    // Set: hash elem, call kata_rt_set_contains(set, elem, hash, eq_fn)
                    let item_i64 = bitcast_to_i64(item_val, ctx);
                    let hash_name = hash_fn_name(t)?;
                    let eq_name = eq_fn_name(t)?;
                    let hash_ref =
                        ctx.ffi_refs.get(hash_name).copied().ok_or_else(|| {
                            super::CodegenError::FfiSymbolNotFound(hash_name.into())
                        })?;
                    let contains_ref = ctx
                        .ffi_refs
                        .get("kata_rt_set_contains")
                        .copied()
                        .ok_or_else(|| {
                            super::CodegenError::FfiSymbolNotFound("kata_rt_set_contains".into())
                        })?;
                    let hash_call = ctx.builder.ins().call(hash_ref, &[item_i64]);
                    let hash_val = ctx.builder.inst_results(hash_call)[0];
                    let eq_fn_ptr = get_ffi_fn_ptr(eq_name, ctx)?;
                    let call = ctx
                        .builder
                        .ins()
                        .call(contains_ref, &[coll_val, item_i64, hash_val, eq_fn_ptr]);
                    Ok(Some(ctx.builder.inst_results(call)[0]))
                }
                _ => Err(super::CodegenError::UnsupportedNode(format!(
                    "In sobre tipo não-coleção: {coll_ty:?}"
                ))),
            }
        }

        // ── DictLit — constrói dict via kata_rt_dict_empty + insert chain ──
        TypedExprKind::DictLit {
            entries,
            key_ty,
            value_ty,
        } => {
            let val = lower_dict_lit(entries, key_ty, value_ty, expr, ctx)?;
            Ok(Some(val))
        }

        // ── SetLit — constrói set via kata_rt_set_empty + insert chain ──
        TypedExprKind::SetLit { elements, elem_ty } => {
            let val = lower_set_lit(elements, elem_ty, expr, ctx)?;
            Ok(Some(val))
        }

        _ => Ok(None),
    }
}

// ── Helpers for Dict/Set lowering ─────────────────────────────────────────

/// Resolve o hash FFI function name para um dado key type.
pub(crate) fn hash_fn_name(key_ty: &Ty) -> Result<&'static str, super::CodegenError> {
    match key_ty {
        Ty::Prim(PrimTy::Int) => Ok("kata_rt_hash_int"),
        Ty::Prim(PrimTy::Text) => Ok("kata_rt_hash_text"),
        Ty::Prim(PrimTy::Rational) => Ok("kata_rt_hash_rational"),
        _ => Err(super::CodegenError::UnsupportedNode(format!(
            "DictLit/SetLit: tipo de chave não-hashable: {key_ty}"
        ))),
    }
}

/// Resolve o eq FFI function name para um dado key type.
pub(crate) fn eq_fn_name(key_ty: &Ty) -> Result<&'static str, super::CodegenError> {
    match key_ty {
        Ty::Prim(PrimTy::Int) => Ok("kata_rt_bi_eq"),
        Ty::Prim(PrimTy::Text) => Ok("kata_rt_string_eq"),
        _ => Err(super::CodegenError::UnsupportedNode(format!(
            "DictLit/SetLit: tipo de chave sem eq_fn: {key_ty}"
        ))),
    }
}

/// Obtém o ponteiro de uma função FFI como valor i64 (via GlobalValue::Symbol).
///
/// Segue o mesmo padrão usado em `action_call.rs` para obter function pointers:
/// 1. `declare_func_in_func` → FuncRef
/// 2. `ext_funcs[func_ref].name` → ExternalName
/// 3. `create_global_value(GlobalValueData::Symbol { name, ... })` → GlobalValue
/// 4. `global_value(pointer_type, gv)` → Value (fn_ptr)
pub(crate) fn get_ffi_fn_ptr(
    ffi_name: &str,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let fid = ctx
        .ffi_ids
        .get(ffi_name)
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound(ffi_name.to_string()))?;
    let func_ref = ctx.module.declare_func_in_func(*fid, ctx.builder.func);
    let ext_func_name = ctx.builder.func.dfg.ext_funcs[func_ref].name.clone();
    let func_gv = ctx
        .builder
        .func
        .create_global_value(GlobalValueData::Symbol {
            name: ext_func_name,
            offset: 0.into(),
            // FFI imports are in the host binary, not colocated with JIT code.
            // colocated: true → PC-relative (PCRel4) → i32 overflow if too far.
            // colocated: false → absolute (Abs8) → works regardless of distance.
            colocated: false,
            tls: false,
        });
    Ok(ctx
        .builder
        .ins()
        .global_value(ctx.module.target_config().pointer_type(), func_gv))
}

/// Bitcast F64→I64 se necessário (mesmo pattern do ArrayLit/ListLit).
pub(crate) fn bitcast_to_i64(
    val: cranelift_codegen::ir::Value,
    ctx: &mut LowerCtx,
) -> cranelift_codegen::ir::Value {
    let ty = ctx.builder.func.dfg.value_type(val);
    if ty == cranelift_codegen::ir::types::F64 {
        ctx.builder.ins().bitcast(I64, MemFlagsData::new(), val)
    } else {
        val
    }
}

/// Lowera `DictLit { entries, key_ty, value_ty }`:
///
/// 1. dict = kata_rt_dict_empty(arena)
/// 2. Para cada (key, val):
///    a. hash = hash_fn(key)
///    b. eq_fn_ptr = get_ffi_fn_ptr(eq_fn_name)
///    c. dict = kata_rt_dict_insert(dict, key, val, hash, eq_fn_ptr, arena)
/// 3. Retorna dict
fn lower_dict_lit(
    entries: &[(kata_ast::Spanned<TypedExpr>, kata_ast::Spanned<TypedExpr>)],
    key_ty: &Ty,
    _value_ty: &Ty,
    _expr: &TypedExpr,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let arena_handle = ctx
        .fiber_arena
        .or(ctx.caller_arena)
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // dict = kata_rt_dict_empty(arena)
    let empty_ref = ctx
        .ffi_refs
        .get("kata_rt_dict_empty")
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_dict_empty".into()))?;
    let call = ctx.builder.ins().call(*empty_ref, &[arena_handle]);
    let mut dict = ctx.builder.inst_results(call)[0];

    // Resolve hash e eq function names.
    let hash_name = hash_fn_name(key_ty)?;
    let eq_name = eq_fn_name(key_ty)?;
    let hash_ref = ctx
        .ffi_refs
        .get(hash_name)
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound(hash_name.into()))?;
    let insert_ref = ctx
        .ffi_refs
        .get("kata_rt_dict_insert")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_dict_insert".into()))?;

    // Resolve eq_fn pointer ONCE (outside loop) — creating multiple GlobalValues
    // for the same symbol in a single function body can corrupt the pointer.
    let eq_fn_ptr = get_ffi_fn_ptr(eq_name, ctx)?;

    for (key_expr, val_expr) in entries {
        let key_val = lower_expr(&key_expr.node, ctx)?;
        let key_val = bitcast_to_i64(key_val, ctx);
        let val_val = lower_expr(&val_expr.node, ctx)?;
        let val_val = bitcast_to_i64(val_val, ctx);

        // hash = hash_fn(key)
        let hash_call = ctx.builder.ins().call(hash_ref, &[key_val]);
        let hash_val = ctx.builder.inst_results(hash_call)[0];

        // dict = kata_rt_dict_insert(dict, key, val, hash, eq_fn_ptr, arena)
        let insert_call = ctx.builder.ins().call(
            insert_ref,
            &[dict, key_val, val_val, hash_val, eq_fn_ptr, arena_handle],
        );
        dict = ctx.builder.inst_results(insert_call)[0];
    }

    Ok(dict)
}

/// Lowera `SetLit { elements, elem_ty }`:
///
/// 1. set = kata_rt_set_empty(arena)
/// 2. Para cada elem:
///    a. hash = hash_fn(elem)
///    b. eq_fn_ptr = get_ffi_fn_ptr(eq_fn_name)
///    c. set = kata_rt_set_insert(set, elem, hash, eq_fn_ptr, arena)
/// 3. Retorna set
fn lower_set_lit(
    elements: &[kata_ast::Spanned<TypedExpr>],
    elem_ty: &Ty,
    _expr: &TypedExpr,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let arena_handle = ctx
        .fiber_arena
        .or(ctx.caller_arena)
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // set = kata_rt_set_empty(arena)
    let empty_ref = ctx
        .ffi_refs
        .get("kata_rt_set_empty")
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_set_empty".into()))?;
    let call = ctx.builder.ins().call(*empty_ref, &[arena_handle]);
    let mut set = ctx.builder.inst_results(call)[0];

    // Resolve hash e eq function names.
    let hash_name = hash_fn_name(elem_ty)?;
    let eq_name = eq_fn_name(elem_ty)?;
    let hash_ref = ctx
        .ffi_refs
        .get(hash_name)
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound(hash_name.into()))?;
    let insert_ref = ctx
        .ffi_refs
        .get("kata_rt_set_insert")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_set_insert".into()))?;

    // Resolve eq_fn pointer ONCE (outside loop).
    let eq_fn_ptr = get_ffi_fn_ptr(eq_name, ctx)?;

    for elem in elements {
        let elem_val = lower_expr(&elem.node, ctx)?;
        let elem_val = bitcast_to_i64(elem_val, ctx);

        // hash = hash_fn(elem)
        let hash_call = ctx.builder.ins().call(hash_ref, &[elem_val]);
        let hash_val = ctx.builder.inst_results(hash_call)[0];

        // set = kata_rt_set_insert(set, elem, hash, eq_fn_ptr, arena)
        let insert_call = ctx.builder.ins().call(
            insert_ref,
            &[set, elem_val, hash_val, eq_fn_ptr, arena_handle],
        );
        set = ctx.builder.inst_results(insert_call)[0];
    }

    Ok(set)
}
