//! Lowering de closures — arm `Closure` do match `lower_expr` + helper `alloc_capture_box`.
//!
//! Extraído de `expr.rs` para reduzir o tamanho do dispatch central.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Signature};
use cranelift_codegen::isa::CallConv;
use kata_core::ty::Ty;
use kata_inference::{CaptureInfo, TypedExpr, TypedExprKind};

use super::LowerCtx;

/// Arena handle a ser passada como primeiro param implícito para funções Kata
/// e lambdas. Prefere fiber_arena (arena local do fiber), fallback caller_arena.
fn caller_arena_handle(ctx: &mut LowerCtx) -> cranelift_codegen::ir::Value {
    ctx.fiber_arena
        .or(ctx.caller_arena)
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0))
}

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
        // Como funções sintetizadas têm nomes únicos (ex: repr__Pessoa),
        // fazemos lookup por chave composta usando o sym_name como nome
        // e os tipos do callee.
        let key = super::func_key_from_callee(callee)
            .map(|(_name, params, ret)| (sym_name.clone(), params, ret))
            .unwrap_or_else(|_| (sym_name.clone(), Vec::new(), Ty::Unit));
        if let Some(&func_ref) = ctx.kata_refs.get(&key) {
            // ABI A2: rt + arena_handle + box_ptr (dummy 0) sempre presentes.
            let rt_val = ctx.rt.unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
            let arena = caller_arena_handle(ctx);
            let dummy_box = ctx.builder.ins().iconst(I64, 0);
            let mut kata_args = vec![rt_val, arena, dummy_box];
            kata_args.extend_from_slice(&arg_values);
            let call_inst = ctx.builder.ins().call(func_ref, &kata_args);
            let results = ctx.builder.inst_results(call_inst);
            if results.is_empty() {
                return Ok(ctx.builder.ins().iconst(I64, 0));
            }
            return Ok(results[0]);
        }
        let func_ref =
            ctx.ffi_refs
                .get(sym_name)
                .ok_or_else(|| super::CodegenError::FfiSymbolNotFound {
                    symbol: sym_name.clone(),
                })?;
        // FFIs que alocam na arena (cons, concat, reverse, array_alloc, etc.)
        // esperam arena_handle como último param, mas o caller não fornece.
        // Injetar automaticamente.
        let mut call_args = arg_values;

        // ── Dict/Set FFI interception (Fio 13) ──
        // These FFI functions need extra params (hash, eq_fn, arena) that
        // aren't in the Kata-level signatures. Intercept and inject them.
        let arena_for_dict = ctx
            .fiber_arena
            .or(ctx.caller_arena)
            .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

        match sym_name.as_str() {
            "kata_rt_dict_get_checked" => {
                // Args: [dict, key] → [dict, key, hash(key), eq_fn, arena]
                let key_ty = &args[1].node.ty;
                let key_val = super::dict_set_lit::bitcast_to_i64(call_args[1], ctx);
                call_args[1] = key_val;
                let hash_name = super::dict_set_lit::hash_fn_name(key_ty)?;
                let eq_name = super::dict_set_lit::eq_fn_name(key_ty)?;
                let hash_ref = ctx.ffi_refs.get(hash_name).copied().ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound {
                        symbol: hash_name.into(),
                    }
                })?;
                let hash_call = ctx.builder.ins().call(hash_ref, &[key_val]);
                let hash_val = ctx.builder.inst_results(hash_call)[0];
                let eq_fn_ptr = super::dict_set_lit::get_ffi_fn_ptr(eq_name, ctx)?;
                call_args.push(hash_val);
                call_args.push(eq_fn_ptr);
                call_args.push(arena_for_dict);
            }
            "kata_rt_dict_insert" => {
                // Args: [dict, key, val] → [dict, key, val, hash(key), eq_fn, arena]
                // key is arg_values[1], need its type from args[1].node.ty
                let key_ty = &args[1].node.ty;
                let key_val = super::dict_set_lit::bitcast_to_i64(call_args[1], ctx);
                call_args[1] = key_val;
                let hash_name = super::dict_set_lit::hash_fn_name(key_ty)?;
                let eq_name = super::dict_set_lit::eq_fn_name(key_ty)?;
                let hash_ref = ctx.ffi_refs.get(hash_name).copied().ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound {
                        symbol: hash_name.into(),
                    }
                })?;
                let hash_call = ctx.builder.ins().call(hash_ref, &[key_val]);
                let hash_val = ctx.builder.inst_results(hash_call)[0];
                let eq_fn_ptr = super::dict_set_lit::get_ffi_fn_ptr(eq_name, ctx)?;
                call_args.push(hash_val);
                call_args.push(eq_fn_ptr);
                call_args.push(arena_for_dict);
            }
            "kata_rt_dict_remove" => {
                // Args: [dict, key] → [dict, key, hash(key), eq_fn, arena]
                let key_ty = &args[1].node.ty;
                let key_val = super::dict_set_lit::bitcast_to_i64(call_args[1], ctx);
                call_args[1] = key_val;
                let hash_name = super::dict_set_lit::hash_fn_name(key_ty)?;
                let eq_name = super::dict_set_lit::eq_fn_name(key_ty)?;
                let hash_ref = ctx.ffi_refs.get(hash_name).copied().ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound {
                        symbol: hash_name.into(),
                    }
                })?;
                let hash_call = ctx.builder.ins().call(hash_ref, &[key_val]);
                let hash_val = ctx.builder.inst_results(hash_call)[0];
                let eq_fn_ptr = super::dict_set_lit::get_ffi_fn_ptr(eq_name, ctx)?;
                call_args.push(hash_val);
                call_args.push(eq_fn_ptr);
                call_args.push(arena_for_dict);
            }
            "kata_rt_set_union" | "kata_rt_set_intersection" | "kata_rt_set_difference" => {
                // Args: [a, b] → [a, b, eq_fn, arena]
                // Element type from args[0].node.ty (Set::T → T)
                let elem_ty = match &args[0].node.ty {
                    Ty::Set(inner) => inner.as_ref().clone(),
                    other => {
                        return Err(super::CodegenError::UnsupportedNode {
                            node: format!("set op on non-Set type: {other}"),
                        });
                    }
                };
                let eq_name = super::dict_set_lit::eq_fn_name(&elem_ty)?;
                let eq_fn_ptr = super::dict_set_lit::get_ffi_fn_ptr(eq_name, ctx)?;
                call_args.push(eq_fn_ptr);
                call_args.push(arena_for_dict);
            }
            "kata_rt_dict_merge" => {
                // Args: [a, b] → [a, b, eq_fn, arena]
                // Key type from args[0].node.ty (Dict::(K, V) → K)
                let key_ty = match &args[0].node.ty {
                    Ty::Dict(k, _) => k.as_ref().clone(),
                    other => {
                        return Err(super::CodegenError::UnsupportedNode {
                            node: format!("dict_merge on non-Dict type: {other}"),
                        });
                    }
                };
                let eq_name = super::dict_set_lit::eq_fn_name(&key_ty)?;
                let eq_fn_ptr = super::dict_set_lit::get_ffi_fn_ptr(eq_name, ctx)?;
                call_args.push(eq_fn_ptr);
                call_args.push(arena_for_dict);
            }
            "kata_rt_file_open" => {
                // FileOpen precisa de arena baseada em escape analysis,
                // não fiber_arena fixo. Local → fiber_arena, Caller →
                // caller_arena, Heap → root_arena.
                let arena =
                    crate::lowering::escape_arena::arena_handle_for_escape(expr.escape, ctx);
                call_args.push(arena);
            }
            _ => {
                // Default: inject arena if needed (existing behavior)
                if crate::ffi_sigs::ffi_needs_arena(sym_name) {
                    let arena = ctx
                        .fiber_arena
                        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
                    call_args.push(arena);
                }
            }
        }
        let call_inst = ctx.builder.ins().call(*func_ref, &call_args);
        // Void FFI (ex: kata_rt_log_config) — sem retorno. Retorna Unit (iconst 0).
        let results = ctx.builder.inst_results(call_inst);
        if results.is_empty() {
            Ok(ctx.builder.ins().iconst(I64, 0))
        } else {
            Ok(results[0])
        }
    } else {
        // ffi_symbol = None: função Kata nomeada ou lambda como valor.
        // Sub-caminho A: callee é Lambda inline (use-site inference).
        // O lambda foi deferido e re-inferido no site de uso. Compila
        // o lambda JIT, extrai fn_ptr do box_ptr, e faz call_indirect.
        if let TypedExprKind::Lambda { .. } = &callee.node.kind {
            // Lowera o lambda — compila a função JIT e retorna box_ptr.
            let box_ptr = super::expr::lower_expr(&callee.node, ctx)?;
            // Carrega fn_ptr do box_ptr (offset 0 do CaptureBox).
            let flags = MemFlagsData::new();
            let func_ptr = ctx.builder.ins().load(I64, flags, box_ptr, 0);
            // A2: Primeiros params implícitos: rt + arena_handle + box_ptr.
            let rt_val = ctx.rt.unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
            let arena = caller_arena_handle(ctx);
            let mut call_args = vec![rt_val, arena, box_ptr];
            call_args.extend(arg_values.iter().copied());
            // Constrói a assinatura para call_indirect.
            let callee_ty = &callee.node.ty;
            if let Ty::Function(param_types, ret_ty) = callee_ty {
                let mut sig = Signature::new(CallConv::Tail);
                sig.params.push(AbiParam::new(I64)); // rt (A2)
                sig.params.push(AbiParam::new(I64)); // arena_handle
                sig.params.push(AbiParam::new(I64)); // box_ptr
                for pt in param_types {
                    sig.params.push(AbiParam::new(super::resolve_clif_ty(
                        pt,
                        ctx.struct_registry,
                    )));
                }
                sig.returns.push(AbiParam::new(super::resolve_clif_ty(
                    ret_ty,
                    ctx.struct_registry,
                )));
                let sig_ref = ctx.builder.func.import_signature(sig);
                if expr.tail_pos && !ctx.no_tail_calls {
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
                return Ok(result);
            }
            // Se o callee não é Function, não há como despachar.
            return Err(super::CodegenError::UnsupportedNode {
                node: format!(
                    "Closure com callee Lambda não-Function: {:?}",
                    callee.node.ty
                ),
            });
        }
        // Sub-caminho B: callee é Ident — função Kata nomeada ou variável.
        // Tenta Kata function call direto primeiro.
        if let TypedExprKind::Ident { name } = &callee.node.kind {
            // Lookup por chave composta (name, param_types, ret_ty) extraída do callee.
            if let Ok(key) = super::func_key_from_callee(callee)
                && let Some(&func_ref) = ctx.kata_refs.get(&key)
            {
                // Call direto para função Kata nomeada.
                // ABI A2: rt + arena_handle + box_ptr (dummy 0) sempre presentes.
                if expr.tail_pos && !ctx.no_tail_calls {
                    let rt_val = ctx.rt.unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
                    let arena = caller_arena_handle(ctx);
                    let dummy_box = ctx.builder.ins().iconst(I64, 0);
                    let mut tail_args = vec![rt_val, arena, dummy_box];
                    tail_args.extend(arg_values.iter().copied());
                    ctx.builder.ins().return_call(func_ref, &tail_args);
                    ctx.emitted_tail_call = true;
                    let dummy = ctx.builder.create_block();
                    ctx.builder.switch_to_block(dummy);
                    ctx.builder.seal_block(dummy);
                    let val = ctx.builder.ins().iconst(I64, 0);
                    ctx.builder.ins().return_(&[val]);
                    return Ok(val);
                }
                let rt_val = ctx.rt.unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
                let arena = caller_arena_handle(ctx);
                let dummy_box = ctx.builder.ins().iconst(I64, 0);
                let mut call_args = vec![rt_val, arena, dummy_box];
                call_args.extend(arg_values.iter().copied());
                let call_inst = ctx.builder.ins().call(func_ref, &call_args);
                return Ok(ctx.builder.inst_results(call_inst)[0]);
            }
            // Ident não está no kata_refs: pode ser variável com
            // Ty::Function (lambda como valor) — call_indirect.
            // ABI uniformizada: o valor em var_map é box_ptr (não fn_ptr).
            if let Some(var) = ctx.var_map.get(name) {
                let box_ptr = ctx.builder.use_var(*var);
                // Carrega fn_ptr do box_ptr (offset 0 do CaptureBox).
                let flags = MemFlagsData::new();
                let func_ptr = ctx.builder.ins().load(I64, flags, box_ptr, 0);

                // A2: Primeiros params implícitos: rt + arena_handle + box_ptr.
                let rt_val = ctx.rt.unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
                let arena = caller_arena_handle(ctx);
                let mut call_args = vec![rt_val, arena, box_ptr];
                call_args.extend(arg_values.iter().copied());

                // Constrói a assinatura para call_indirect.
                let callee_ty = &callee.node.ty;
                if let Ty::Function(param_types, ret_ty) = callee_ty {
                    let mut sig = Signature::new(CallConv::Tail);
                    sig.params.push(AbiParam::new(I64)); // rt (A2)
                    sig.params.push(AbiParam::new(I64)); // arena_handle
                    sig.params.push(AbiParam::new(I64)); // box_ptr
                    for pt in param_types {
                        sig.params.push(AbiParam::new(super::resolve_clif_ty(
                            pt,
                            ctx.struct_registry,
                        )));
                    }
                    sig.returns.push(AbiParam::new(super::resolve_clif_ty(
                        ret_ty,
                        ctx.struct_registry,
                    )));
                    let sig_ref = ctx.builder.func.import_signature(sig);
                    if expr.tail_pos && !ctx.no_tail_calls {
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
                    return Ok(result);
                }
            }
        }
        Err(super::CodegenError::UnsupportedNode {
            node: format!(
                "Closure sem ffi_symbol e callee não-Ident: {:?}",
                callee.node.kind
            ),
        })
    }
}

/// Aloca um CaptureBox na arena global e retorna o ponteiro.
///
/// 1. Aloca um array temporário de `n_captures * 8` bytes na arena disponível.
/// 2. Preenche o array com os valores das captures (lidos do var_map).
/// 3. Chama `kata_rt_alloc_arc(fn_ptr, array_ptr, n_captures, arena)` → `box_ptr`.
///
/// O CaptureBox contém: fn_ptr (offset 0), refcount=1 (offset 8),
/// n_captures (offset 16), captures[0..n] (offset 24+).
pub(crate) fn alloc_capture_box(
    func_ptr: cranelift_codegen::ir::Value,
    captures: &[CaptureInfo],
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let flags = MemFlagsData::new();

    let rt_val = ctx.rt.unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    let get_root_ref = ctx
        .ffi_refs
        .get("kata_rt_get_root_arena_handle")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound {
            symbol: "kata_rt_get_root_arena_handle".into(),
        })?;
    let root_inst = ctx.builder.ins().call(get_root_ref, &[rt_val]);
    let capture_arena = ctx.builder.inst_results(root_inst)[0];

    let alloc_arc_ref = ctx.ffi_refs.get("kata_rt_alloc_arc").ok_or_else(|| {
        super::CodegenError::FfiSymbolNotFound {
            symbol: "kata_rt_alloc_arc".into(),
        }
    })?;

    if captures.is_empty() {
        // Sem captures: cria box com fn_ptr e n_captures=0, sem alocar array.
        let null_array = ctx.builder.ins().iconst(I64, 0);
        let n_val = ctx.builder.ins().iconst(I64, 0);
        let arc_inst = ctx.builder.ins().call(
            *alloc_arc_ref,
            &[rt_val, func_ptr, null_array, n_val, capture_arena],
        );
        return Ok(ctx.builder.inst_results(arc_inst)[0]);
    }

    // Com captures: aloca array, preenche, e cria box.
    let n = captures.len() as i64;
    let arena_alloc_ref = ctx.ffi_refs.get("kata_rt_arena_alloc").ok_or_else(|| {
        super::CodegenError::FfiSymbolNotFound {
            symbol: "kata_rt_arena_alloc".into(),
        }
    })?;
    let array_size = ctx.builder.ins().iconst(I64, n * 8);
    let alloc_inst = ctx
        .builder
        .ins()
        .call(*arena_alloc_ref, &[rt_val, capture_arena, array_size]);
    let array_ptr = ctx.builder.inst_results(alloc_inst)[0];

    for (i, cap) in captures.iter().enumerate() {
        let cap_var =
            ctx.var_map
                .get(&cap.name)
                .ok_or_else(|| super::CodegenError::UnsupportedNode {
                    node: format!("capture '{}' não encontrada no var_map", cap.name),
                })?;
        let cap_val = ctx.builder.use_var(*cap_var);
        let offset = (i * 8) as i32;
        ctx.builder.ins().store(flags, cap_val, array_ptr, offset);
    }

    let n_val = ctx.builder.ins().iconst(I64, n);
    let arc_inst = ctx.builder.ins().call(
        *alloc_arc_ref,
        &[rt_val, func_ptr, array_ptr, n_val, capture_arena],
    );
    let box_ptr = ctx.builder.inst_results(arc_inst)[0];

    Ok(box_ptr)
}
