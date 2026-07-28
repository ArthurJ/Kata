//! Lowera uma expressão TAST → valor CLIF.
//!
//! Dispatch central — cada variante de `TypedExprKind` é lowerada aqui.
//! Funções que não são do tipo expressão (module, match, clause, pattern)
//! vivem em submódulos irmãos.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, GlobalValueData, InstBuilder, MemFlagsData, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_module::{Linkage, Module};
use kata_core::ty::{PrimTy, Ty};
use kata_inference::{TypedExpr, TypedExprKind};

use super::_match::lower_match;
use super::LowerCtx;
use super::action_call::lower_action_call;
use super::closure::lower_closure;
use super::collections_hof::lower_fold;
use super::collections_literal::lower_collections_literal;
use super::control_flow::lower_control_flow;
use super::filter::lower_filter;
use super::fused_stream::lower_fused_stream;
use super::map::lower_map;
use crate::smi::{encode_smi, fits_smi, parse_int_literal};

/// Lowera uma expressão TAST → valor CLIF.
pub(crate) fn lower_expr(
    expr: &TypedExpr,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    match &expr.kind {
        // ── IntLit: SMI inline ou kata_rt_tag_int_from_str para BigInt ──
        TypedExprKind::IntLit { text } => {
            // Tenta parsear como i64. Se conseguir e couber em SMI, inline.
            // Se não (BigInt grande), chama kata_rt_tag_int_from_str.
            let parsed = parse_int_literal(text);
            if let Some(val) = parsed {
                if fits_smi(val) {
                    // SMI: (val << 1) | 1 — inline, sem FFI call.
                    return Ok(ctx.builder.ins().iconst(I64, encode_smi(val)));
                }
                // i64 mas não cabe em SMI — chama kata_rt_tag_int(val).
                let raw = ctx.builder.ins().iconst(I64, val);
                let func_ref = ctx.ffi_refs.get("kata_rt_tag_int").ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_tag_int".into())
                })?;
                let call_inst = ctx.builder.ins().call(*func_ref, &[raw]);
                return Ok(ctx.builder.inst_results(call_inst)[0]);
            }
            // BigInt grande: chama kata_rt_tag_int_from_str(ptr, len).
            let global = ctx.add_string(text);
            let ptr = ctx
                .builder
                .ins()
                .global_value(ctx.module.target_config().pointer_type(), global);
            let len = ctx.builder.ins().iconst(I64, text.len() as i64);
            let func_ref = ctx
                .ffi_refs
                .get("kata_rt_tag_int_from_str")
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_tag_int_from_str".into())
                })?;
            let call_inst = ctx.builder.ins().call(*func_ref, &[ptr, len]);
            Ok(ctx.builder.inst_results(call_inst)[0])
        }

        // ── FloatLit: f64 const direto ──
        TypedExprKind::FloatLit { text } => {
            let val: f64 = text.parse().unwrap_or(f64::NAN);
            Ok(ctx.builder.ins().f64const(val))
        }

        // ── TextLit: string alocada como data symbol ──
        TypedExprKind::TextLit { text } => {
            let global = ctx.add_string(text);
            let ptr = ctx
                .builder
                .ins()
                .global_value(ctx.module.target_config().pointer_type(), global);
            Ok(ptr)
        }

        // ── Unit: i64 zero ──
        TypedExprKind::Unit => Ok(ctx.builder.ins().iconst(I64, 0)),

        // ── Ident: use_var (variável local) ou function pointer (função nomeada) ──
        TypedExprKind::Ident { name } => {
            // Caminho 1: variável local no var_map (let bindings, parâmetros).
            if let Some(var) = ctx.var_map.get(name) {
                return Ok(ctx.builder.use_var(*var));
            }
            // Caminho 2: função Kata nomeada — carrega o function pointer
            // via GlobalValue (mesmo mecanismo do TypedExprKind::Lambda com
            // func_name = Some). Permite `let g := fat` → g carrega ptr de fat.
            // Lookup por chave composta: (name, params, ret) extraída de expr.ty.
            if let Ty::Function(params, ret) = &expr.ty {
                let key = (name.clone(), params.clone(), (**ret).clone());
                if let Some(&func_id) = ctx.kata_ids.get(&key) {
                    let func_ref = ctx.module.declare_func_in_func(func_id, ctx.builder.func);
                    let ext_func_name = ctx.builder.func.dfg.ext_funcs[func_ref].name.clone();
                    let func_gv = ctx
                        .builder
                        .func
                        .create_global_value(GlobalValueData::Symbol {
                            name: ext_func_name,
                            offset: 0.into(),
                            colocated: true,
                            tls: false,
                        });
                    let func_ptr = ctx
                        .builder
                        .ins()
                        .global_value(ctx.module.target_config().pointer_type(), func_gv);
                    // ABI uniformizada: função nomeada como valor também é
                    // box_ptr (sem captures — n_captures=0).
                    let box_ptr = super::closure::alloc_capture_box(func_ptr, &[], ctx)?;
                    return Ok(box_ptr);
                }
            }
            // Caminho 3: Action first-class reference (Ty::Action).
            // Mesmo mecanismo que Ty::Function — obtém fn_ptr via GlobalValue::Symbol.
            if let Ty::Action(params, ret) = &expr.ty {
                let key = (name.clone(), params.clone(), (**ret).clone());
                if let Some(&func_id) = ctx.kata_ids.get(&key) {
                    let func_ref = ctx.module.declare_func_in_func(func_id, ctx.builder.func);
                    let ext_func_name = ctx.builder.func.dfg.ext_funcs[func_ref].name.clone();
                    let func_gv = ctx
                        .builder
                        .func
                        .create_global_value(GlobalValueData::Symbol {
                            name: ext_func_name,
                            offset: 0.into(),
                            colocated: true,
                            tls: false,
                        });
                    return Ok(ctx
                        .builder
                        .ins()
                        .global_value(ctx.module.target_config().pointer_type(), func_gv));
                }
            }
            Err(super::CodegenError::UnsupportedNode(format!(
                "unbound ident: {name}"
            )))
        }

        // ── Closure: call FFI, call direto (Kata), ou call_indirect ──
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => lower_closure(expr, callee, args, ffi_symbol, ctx),

        // ── TypeAscription: inspeciona (inner.kind, target_ty) ──
        TypedExprKind::TypeAscription { expr, target_ty } => {
            let inner = &expr.node;
            match (&inner.kind, target_ty) {
                // IntLit → Float: reinterpretar como f64 const.
                (TypedExprKind::IntLit { text }, Ty::Prim(PrimTy::Float)) => {
                    let val: f64 = text.parse().unwrap_or(f64::NAN);
                    Ok(ctx.builder.ins().f64const(val))
                }
                // IntLit → Rational: chama kata_rt_rat_literal.
                (TypedExprKind::IntLit { text }, Ty::Prim(PrimTy::Rational)) => {
                    let global = ctx.add_string(text);
                    let ptr = ctx
                        .builder
                        .ins()
                        .global_value(ctx.module.target_config().pointer_type(), global);
                    let len = ctx.builder.ins().iconst(I64, text.len() as i64);
                    let func_ref = ctx.ffi_refs.get("kata_rt_rat_literal").ok_or_else(|| {
                        super::CodegenError::FfiSymbolNotFound("kata_rt_rat_literal".into())
                    })?;
                    let call_inst = ctx.builder.ins().call(*func_ref, &[ptr, len]);
                    Ok(ctx.builder.inst_results(call_inst)[0])
                }
                // FloatLit → Rational: chama kata_rt_rat_literal.
                (TypedExprKind::FloatLit { text }, Ty::Prim(PrimTy::Rational)) => {
                    let global = ctx.add_string(text);
                    let ptr = ctx
                        .builder
                        .ins()
                        .global_value(ctx.module.target_config().pointer_type(), global);
                    let len = ctx.builder.ins().iconst(I64, text.len() as i64);
                    let func_ref = ctx.ffi_refs.get("kata_rt_rat_literal").ok_or_else(|| {
                        super::CodegenError::FfiSymbolNotFound("kata_rt_rat_literal".into())
                    })?;
                    let call_inst = ctx.builder.ins().call(*func_ref, &[ptr, len]);
                    Ok(ctx.builder.inst_results(call_inst)[0])
                }
                // Mesmo tipo (no-op): lowerar inner.
                _ if inner.ty == *target_ty => lower_expr(inner, ctx),
                // Ascription-refined `5::PositiveInt` — o typeck já
                // validou os predicados em compile-time. Em runtime, o valor
                // é o mesmo do tipo base (alias). Lowerar inner diretamente.
                (TypedExprKind::IntLit { .. }, Ty::Struct(_))
                | (TypedExprKind::FloatLit { .. }, Ty::Struct(_)) => lower_expr(inner, ctx),
                // Downcast refined/alias→base (ex: PositiveInt → Int,
                // Altura → Float). No-op em runtime — mesmos bits.
                // O typeck já validou que o Struct é alias do target.
                // Quando o Cranelift type difere (Struct=I64, Float=F64),
                // precisa bitcast.
                _ if matches!(inner.ty, Ty::Struct(_)) && inner.ty != *target_ty => {
                    let val = lower_expr(inner, ctx)?;
                    let target_clif = super::resolve_clif_ty(target_ty, ctx.struct_registry);
                    let inner_clif = super::resolve_clif_ty(&inner.ty, ctx.struct_registry);
                    if target_clif != inner_clif {
                        // Bitcast necessário (ex: I64 → F64 para Float)
                        Ok(ctx
                            .builder
                            .ins()
                            .bitcast(target_clif, MemFlagsData::new(), val))
                    } else {
                        Ok(val)
                    }
                }
                // Demais casos: o typeck já deveria ter rejeitado.
                _ => Err(super::CodegenError::UnsupportedNode(format!(
                    "ascription não suportada: {:?} → {}",
                    inner.kind, target_ty
                ))),
            }
        }

        // ── Grouping: transparente ──
        TypedExprKind::Grouping { inner } => lower_expr(&inner.node, ctx),

        // ── Tuple: aloca N×8 bytes na arena, store de cada elemento ──
        TypedExprKind::Tuple { elements } => {
            let n = elements.len();
            if n == 0 {
                // Tupla vazia = Unit (zero-sized). Retorna 0.
                return Ok(ctx.builder.ins().iconst(I64, 0));
            }

            // Aloca N * 8 bytes. Para Heap, usa alloc_tracked com header ARC.
            let ptr =
                crate::lowering::escape_arena::alloc_for_escape(expr.escape, (n * 8) as i64, ctx)?;

            // Store de cada elemento no offset i * 8.
            let flags = MemFlagsData::new();
            for (i, elem) in elements.iter().enumerate() {
                let val = lower_expr(&elem.node, ctx)?;
                let offset = (i * 8) as i32;
                ctx.builder.ins().store(flags, val, ptr, offset);
            }

            Ok(ptr)
        }

        // ── StructConstruct: aloca N×8 bytes, store por campo ──
        // Idêntico ao codegen de Tuple no layout — só muda identidade nominal.
        TypedExprKind::StructConstruct {
            struct_name: _,
            values,
        } => {
            let n = values.len();
            if n == 0 {
                // Struct sem campos = zero-sized. Retorna 0.
                return Ok(ctx.builder.ins().iconst(I64, 0));
            }

            // Aloca N * 8 bytes. Para Heap, usa alloc_tracked com header ARC.
            let ptr =
                crate::lowering::escape_arena::alloc_for_escape(expr.escape, (n * 8) as i64, ctx)?;

            // Store de cada campo no offset i * 8.
            let flags = MemFlagsData::new();
            for (i, val_expr) in values.iter().enumerate() {
                let val = lower_expr(&val_expr.node, ctx)?;
                let offset = (i * 8) as i32;
                ctx.builder.ins().store(flags, val, ptr, offset);
            }

            Ok(ptr)
        }

        // ── FieldAccess: load ptr + field_index * 8 ──
        TypedExprKind::FieldAccess {
            expr: inner,
            field_index,
            ..
        } => {
            let ptr = lower_expr(&inner.node, ctx)?;
            let flags = MemFlagsData::new();
            let offset = (*field_index as i32) * 8;
            // Carrega com o tipo CLIF correto do campo (expr.ty).
            // Float mapeia para F64; sem isso, carrega como I64 e quebra
            // a verificação de tipos do Cranelift em chamadas FFI.
            let clif_ty = super::resolve_clif_ty(&expr.ty, ctx.struct_registry);
            Ok(ctx.builder.ins().load(clif_ty, flags, ptr, offset))
        }

        // ── IndexAccess: load ptr + element_index * 8 ──
        TypedExprKind::IndexAccess {
            expr: inner,
            element_index,
            ..
        } => {
            let ptr = lower_expr(&inner.node, ctx)?;
            let flags = MemFlagsData::new();
            let offset = (*element_index as i32) * 8;
            let clif_ty = super::resolve_clif_ty(&expr.ty, ctx.struct_registry);
            Ok(ctx.builder.ins().load(clif_ty, flags, ptr, offset))
        }

        // ── Let: definir variável ──
        TypedExprKind::Let { name, value } => {
            // ABI uniformizada: se o value é um Lambda, alloc_capture_box
            // já foi chamado no arm Lambda — o valor é box_ptr.
            // closure_captures não é mais necessário.
            let val = lower_expr(&value.node, ctx)?;
            let clif_ty = super::resolve_clif_ty(&value.node.ty, ctx.struct_registry);
            let var = ctx.new_var(name, clif_ty);
            ctx.builder.def_var(var, val);
            // Se o valor é Heap (ARC-managed), registrar para decref no epílogo.
            if value.node.escape == kata_core::escape::EscapeTarget::Heap {
                ctx.arc_vars.push(var);
            }
            // Let retorna Unit.
            Ok(ctx.builder.ins().iconst(I64, 0))
        }
        // ── LetDestruct: let (x, y, ...) := expr ──
        // 1. Avalia expr e define no temp_name
        // 2. Para cada binding, faz FieldAccess(temp, i) e define
        TypedExprKind::LetDestruct {
            temp_name,
            value,
            bindings,
        } => {
            // ABI uniformizada: captures já são alocadas no arm Lambda.
            let val = lower_expr(&value.node, ctx)?;
            let clif_ty = super::resolve_clif_ty(&value.node.ty, ctx.struct_registry);
            let temp_var = ctx.new_var(temp_name, clif_ty);
            ctx.builder.def_var(temp_var, val);

            // Para cada binding, carrega o elemento via FieldAccess.
            for (name, access_expr) in bindings {
                let elem_val = lower_expr(&access_expr.node, ctx)?;
                let elem_clif_ty = super::resolve_clif_ty(&access_expr.node.ty, ctx.struct_registry);
                let elem_var = ctx.new_var(name, elem_clif_ty);
                ctx.builder.def_var(elem_var, elem_val);
            }

            Ok(ctx.builder.ins().iconst(I64, 0))
        }

        // ── VariantQual: Boolean::True = 1, Boolean::False = 0 ──
        // Para outros enums: VariantQual unitária → kata_rt_store_sum_result(tag, 0).
        TypedExprKind::VariantQual {
            enum_name,
            variant,
            tag,
            ..
        } => super::variant::lower_variant_qual(expr, enum_name, variant, tag, ctx),

        // ── VariantConstruct: Sum com payload ──
        // Result::Ok 42 → kata_rt_store_sum_result(tag, payload) → box_ptr
        TypedExprKind::VariantConstruct {
            enum_name: _,
            variant: _,
            payload,
            tag,
            ..
        } => super::variant::lower_variant_construct(expr, payload, tag, ctx),

        // ── Lambda: função Cranelift separada (anon) ou referência (nomeado) ──
        TypedExprKind::Lambda {
            func_name,
            param_types,
            ret_ty,
            clauses,
            captures,
        } => {
            // Para lambda anônimo (func_name = None): declarar e definir
            // uma função separada no JITModule.
            // Para função nomeada (func_name = Some): já foi definida em
            // lower_module — retornar o function pointer.
            let name = match func_name {
                Some(n) => n.clone(),
                None => ctx.fresh_anon_name(),
            };

            // Declara a função no module (sem definir o corpo ainda).
            // ABI uniformizada: arena_handle + box_ptr sempre presentes.
            let mut sig = Signature::new(CallConv::Tail);
            sig.params.push(AbiParam::new(I64)); // arena_handle
            sig.params.push(AbiParam::new(I64)); // box_ptr
            for pt in param_types {
                sig.params.push(AbiParam::new(super::resolve_clif_ty(pt, ctx.struct_registry)));
            }
            sig.returns.push(AbiParam::new(super::resolve_clif_ty(ret_ty, ctx.struct_registry)));
            let func_id = ctx
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| super::CodegenError::Cranelift(format!("declare fn {name}: {e}")))?;

            // Compila o corpo usando o pipeline compartilhado.
            crate::lowering::function_def::define_function_body(
                &name,
                param_types,
                ret_ty,
                clauses,
                captures,
                &None, // lambdas anônimas não têm @log
                func_id,
                ctx.module,
                ctx.ffi_ids,
                ctx.kata_ids,
                ctx.string_table,
                ctx.struct_registry,
            )?;

            // Obtém o function pointer via GlobalValue.
            let func_ref = ctx.module.declare_func_in_func(func_id, ctx.builder.func);
            let ext_func_name = ctx.builder.func.dfg.ext_funcs[func_ref].name.clone();
            let func_gv = ctx
                .builder
                .func
                .create_global_value(GlobalValueData::Symbol {
                    name: ext_func_name,
                    offset: 0.into(),
                    colocated: true,
                    tls: false,
                });
            let func_ptr = ctx
                .builder
                .ins()
                .global_value(ctx.module.target_config().pointer_type(), func_gv);

            // ABI uniformizada: todo lambda retorna box_ptr (CaptureBox).
            // As captures são alocadas no escopo onde o lambda aparece —
            // onde as variáveis capturadas existem no var_map. Isto resolve
            // o bug de closure escape via return: o box_ptr carrega fn_ptr
            // + captures, independente de quem chama depois.
            let box_ptr = super::closure::alloc_capture_box(func_ptr, captures, ctx)?;
            Ok(box_ptr)
        }

        // ── Match: pattern matching com branch chain ──
        TypedExprKind::Match { scrutinee, arms } => lower_match(scrutinee, arms, ctx),

        // ── ActionCall — scheduler (entry) ou call direto (dentro de Action) ──
        TypedExprKind::ActionCall {
            callee,
            args,
            caller_arena: _,
            ffi_symbol,
            indirect_callee,
        } => lower_action_call(expr, callee, args, ffi_symbol, indirect_callee, ctx),
        // ── Var — mesmo codegen que Let ──
        TypedExprKind::Var { name, value } => {
            let val = lower_expr(&value.node, ctx)?;
            let clif_ty = super::resolve_clif_ty(&value.node.ty, ctx.struct_registry);
            let var = ctx.new_var(name, clif_ty);
            ctx.builder.def_var(var, val);
            // Var retorna Unit (como Let).
            Ok(ctx.builder.ins().iconst(I64, 0))
        }

        // ── Reassign — def_var com novo valor (variável já existe) ──
        TypedExprKind::Reassign { name, value } => {
            let val = lower_expr(&value.node, ctx)?;
            let var = *ctx.var_map.get(name).ok_or_else(|| {
                super::CodegenError::UnsupportedNode(format!(
                    "Reassign: variável `{name}` não encontrada no var_map"
                ))
            })?;
            ctx.builder.def_var(var, val);
            // Reassign retorna Unit.
            Ok(ctx.builder.ins().iconst(I64, 0))
        }

        // ── Return — jump para epilogue_block ──
        // ── Loop, break, continue ──
        // Delegado para `control_flow` — arms Return, Loop, Break, Continue.
        TypedExprKind::Return(_)
        | TypedExprKind::Loop { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue => {
            if let Some(val) = lower_control_flow(expr, ctx)? {
                return Ok(val);
            }
            // Unreachable — lower_control_flow handles all 4 variants above.
            unreachable!("lower_control_flow should handle Return/Loop/Break/Continue")
        }

        // ── Coleções literais — ListLit, ArrayLit, RangeLit, ForIn, In,
        //     DictLit, SetLit ──
        // Delegado para `collections_literal` — arms de coleções literais.
        TypedExprKind::ListLit { .. }
        | TypedExprKind::ArrayLit { .. }
        | TypedExprKind::RangeLit { .. }
        | TypedExprKind::ForIn { .. }
        | TypedExprKind::In { .. }
        | TypedExprKind::DictLit { .. }
        | TypedExprKind::SetLit { .. } => {
            if let Some(val) = lower_collections_literal(expr, ctx)? {
                return Ok(val);
            }
            unreachable!(
                "lower_collections_literal should handle ListLit/ArrayLit/RangeLit/ForIn/In/DictLit/SetLit"
            )
        }

        // ── Map/filter/fold — lowering ──
        TypedExprKind::Map {
            callback,
            collection,
            coll_ty,
            elem_ty,
            ret_ty,
        } => lower_map(callback, collection, coll_ty, elem_ty, ret_ty, ctx),

        TypedExprKind::Filter {
            callback,
            collection,
            coll_ty,
            elem_ty,
            ret_ty,
        } => lower_filter(callback, collection, coll_ty, elem_ty, ret_ty, ctx),

        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            coll_ty,
            elem_ty,
            ret_ty,
        } => lower_fold(callback, initial, collection, coll_ty, elem_ty, ret_ty, ctx),

        // ── FusedStream — stream fusion ──
        TypedExprKind::FusedStream {
            stages,
            source,
            coll_ty,
            source_elem_ty,
            result_elem_ty,
            ret_ty,
        } => lower_fused_stream(
            stages,
            source,
            coll_ty,
            source_elem_ty,
            result_elem_ty,
            ret_ty,
            ctx,
        ),

        // ── CSP — lowering ──
        TypedExprKind::ChannelSend { channel, value } => {
            super::csp::lower_channel_send(channel, value, ctx)
        }
        TypedExprKind::ChannelRecv {
            channel,
            recv_ty,
            bind_name,
        } => super::csp::lower_channel_recv(channel, bind_name, recv_ty, ctx),
        TypedExprKind::ChannelCreate { kind, elem_ty } => {
            super::csp::lower_channel_create(expr, kind, elem_ty, ctx)
        }
        TypedExprKind::ReceiverFactoryCall { factory, elem_ty } => {
            super::csp::lower_receiver_factory_call(expr, factory, elem_ty, ctx)
        }
        TypedExprKind::Fork {
            action_name,
            action_expr,
            args,
        } => super::csp::lower_fork(expr, action_name, action_expr, args, ctx),

        // ── Select — multiplexação de canais com timeout ──
        TypedExprKind::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => super::csp::lower_select(expr, arms, timeout_ms, timeout_body, ctx),

        // ── TypeOf — introspecção compile-time ──
        // Avalia o inner expr (preserva side-effects), descarta o resultado,
        // e produz a string do tipo via Ty::display().
        TypedExprKind::TypeOf { expr: inner } => {
            // 1. Lowerar o inner expr — side-effects acontecem, valor é descartado.
            let _ = lower_expr(&inner.node, ctx)?;
            // 2. Produzir a string do tipo a partir do tipo do inner expr.
            let type_str = inner.node.ty.display();
            // 3. Alocar como string global (mesmo mecanismo de TextLit).
            let global = ctx.add_string(&type_str);
            let ptr = ctx
                .builder
                .ins()
                .global_value(ctx.module.target_config().pointer_type(), global);
            Ok(ptr)
        }
    }
}
