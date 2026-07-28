//! Lowering de arms de coleções literais — `ListLit`, `ArrayLit`, `RangeLit`,
//! `In`, `DictLit`, `SetLit` do match `lower_expr`.
//!
//! Extraído de `expr.rs` para reduzir o tamanho do dispatch central.
//! `ForIn` está em [`for_in`]; helpers de Dict/Set em [`dict_set_lit`].
//! HOFs (Map, Filter, Fold, FusedStream) continuam em seus submódulos próprios
//! (`map.rs`, `filter.rs`, `collections_hof.rs`, `fused_stream.rs`).

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{InstBuilder, MemFlagsData};

use kata_core::ty::Ty;
use kata_inference::{TypedExpr, TypedExprKind};

use super::LowerCtx;
use super::dict_set_lit::{
    bitcast_to_i64, eq_fn_name, get_ffi_fn_ptr, hash_fn_name, lower_dict_lit, lower_set_lit,
};
use super::expr::lower_expr;
use super::for_in::lower_for_in;

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
            let arena_handle =
                crate::lowering::escape_arena::arena_handle_for_escape(expr.escape, ctx);

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
            let arena_handle =
                crate::lowering::escape_arena::arena_handle_for_escape(expr.escape, ctx);

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
            inclusive,
            elem_ty: _,
        } => {
            let arena_handle =
                crate::lowering::escape_arena::arena_handle_for_escape(expr.escape, ctx);

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
            // Store inclusive flag (offset 24) como SMI: 1 = inclusive, 0 = exclusive.
            let incl_val = ctx.builder.ins().iconst(I64, if *inclusive { 3 } else { 1 });
            ctx.builder.ins().store(flags, incl_val, ptr, 24);
            Ok(Some(ptr))
        }

        // ── ForIn — delegado para for_in::lower_for_in ──
        TypedExprKind::ForIn {
            var_name,
            var_ty,
            iterable,
            body,
        } => {
            let unit = lower_for_in(var_name, var_ty, iterable, body, ctx)?;
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
                    // Detecta step negativo e flag inclusive para decidir direção.
                    let flags = MemFlagsData::new();
                    let start = ctx.builder.ins().load(I64, flags, coll_val, 0);
                    let step = ctx.builder.ins().load(I64, flags, coll_val, 8);
                    let end = ctx.builder.ins().load(I64, flags, coll_val, 16);
                    let incl_raw = ctx.builder.ins().load(I64, flags, coll_val, 24);
                    let incl_val = ctx.builder.ins().ushr_imm(incl_raw, 1);

                    // step < 0?
                    let zero_smi = ctx.builder.ins().iconst(I64, 1);
                    let step_neg = ctx.builder.ins().icmp(
                        cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                        step,
                        zero_smi,
                    );
                    // inclusive?
                    let is_inclusive = ctx.builder.ins().icmp_imm(
                        cranelift_codegen::ir::condcodes::IntCC::NotEqual,
                        incl_val,
                        0,
                    );

                    // step >= 0: item >= start AND (item < end OR (inclusive AND item == end))
                    let ge_start = ctx.builder.ins().icmp(
                        cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThanOrEqual,
                        item_val,
                        start,
                    );
                    let lt_end = ctx.builder.ins().icmp(
                        cranelift_codegen::ir::condcodes::IntCC::SignedLessThan,
                        item_val,
                        end,
                    );
                    let eq_end = ctx.builder.ins().icmp(
                        cranelift_codegen::ir::condcodes::IntCC::Equal,
                        item_val,
                        end,
                    );
                    let incl_ok = ctx.builder.ins().band(is_inclusive, eq_end);
                    let in_pos = ctx.builder.ins().bor(lt_end, incl_ok);
                    let result_pos = ctx.builder.ins().band(ge_start, in_pos);

                    // step < 0: item <= start AND (item > end OR (inclusive AND item == end))
                    let le_start = ctx.builder.ins().icmp(
                        cranelift_codegen::ir::condcodes::IntCC::SignedLessThanOrEqual,
                        item_val,
                        start,
                    );
                    let gt_end = ctx.builder.ins().icmp(
                        cranelift_codegen::ir::condcodes::IntCC::SignedGreaterThan,
                        item_val,
                        end,
                    );
                    let in_neg = ctx.builder.ins().bor(gt_end, incl_ok);
                    let result_neg = ctx.builder.ins().band(le_start, in_neg);

                    // Seleciona baseado no sinal do step
                    let result_i8 = ctx.builder.ins().select(step_neg, result_neg, result_pos);
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

        // ── DictLit — delegado para dict_set_lit::lower_dict_lit ──
        TypedExprKind::DictLit {
            entries,
            key_ty,
            value_ty,
        } => {
            let val = lower_dict_lit(entries, key_ty, value_ty, expr, ctx)?;
            Ok(Some(val))
        }

        // ── SetLit — delegado para dict_set_lit::lower_set_lit ──
        TypedExprKind::SetLit { elements, elem_ty } => {
            let val = lower_set_lit(elements, elem_ty, expr, ctx)?;
            Ok(Some(val))
        }

        _ => Ok(None),
    }
}
