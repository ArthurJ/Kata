//! Lowera uma expressão TAST → valor CLIF.
//!
//! Dispatch central — cada variante de `TypedExprKind` é lowerada aqui.
//! Funções que não são do tipo expressão (module, match, clause, pattern)
//! vivem em submódulos irmãos.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, GlobalValueData, InstBuilder, MemFlagsData, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_module::{Linkage, Module};
use kata_core::escape::EscapeTarget;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::{TypedExpr, TypedExprKind};

use super::_match::lower_match;
use super::LowerCtx;
use super::action_call::lower_action_call;
use super::closure::lower_closure;
use super::control_flow::lower_control_flow;
use crate::ffi_sigs::ty_to_clif;
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
                // Fio 6: ascription-refined `5::PositiveInt` — o typeck já
                // validou os predicados em compile-time. Em runtime, o valor
                // é o mesmo do tipo base (alias). Lowerar inner diretamente.
                (TypedExprKind::IntLit { .. }, Ty::Struct(_))
                | (TypedExprKind::FloatLit { .. }, Ty::Struct(_)) => lower_expr(inner, ctx),
                // Demais casos: o typeck já deveria ter rejeitado.
                _ => Err(super::CodegenError::UnsupportedNode(format!(
                    "ascription não suportada: {:?} → {:?}",
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

            // Aloca N * 8 bytes na arena via kata_rt_arena_alloc(handle, size).
            // Escolha de arena baseada em EscapeTarget (Pré-11):
            // - Local → fiber_arena (liberada no epílogo do fiber)
            // - Caller | Ancestor(_) → caller_arena (sobrevive à destruição da local)
            let handle = match expr.escape {
                EscapeTarget::Local => ctx
                    .fiber_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
                EscapeTarget::Caller | EscapeTarget::Ancestor(_) => ctx
                    .caller_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
            };
            let size = ctx.builder.ins().iconst(I64, (n * 8) as i64);
            let func_ref = ctx.ffi_refs.get("kata_rt_arena_alloc").ok_or_else(|| {
                super::CodegenError::FfiSymbolNotFound("kata_rt_arena_alloc".into())
            })?;
            let call_inst = ctx.builder.ins().call(*func_ref, &[handle, size]);
            let ptr = ctx.builder.inst_results(call_inst)[0];

            // Store de cada elemento no offset i * 8.
            let flags = MemFlagsData::new();
            for (i, elem) in elements.iter().enumerate() {
                let val = lower_expr(&elem.node, ctx)?;
                let offset = (i * 8) as i32;
                ctx.builder.ins().store(flags, val, ptr, offset);
            }

            Ok(ptr)
        }

        // ── StructConstruct: Fio 5 — aloca N×8 bytes, store por campo ──
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

            // Arena baseada em EscapeTarget (igual a Tuple).
            let handle = match expr.escape {
                EscapeTarget::Local => ctx
                    .fiber_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
                EscapeTarget::Caller | EscapeTarget::Ancestor(_) => ctx
                    .caller_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
            };
            let size = ctx.builder.ins().iconst(I64, (n * 8) as i64);
            let func_ref = ctx.ffi_refs.get("kata_rt_arena_alloc").ok_or_else(|| {
                super::CodegenError::FfiSymbolNotFound("kata_rt_arena_alloc".into())
            })?;
            let call_inst = ctx.builder.ins().call(*func_ref, &[handle, size]);
            let ptr = ctx.builder.inst_results(call_inst)[0];

            // Store de cada campo no offset i * 8.
            let flags = MemFlagsData::new();
            for (i, val_expr) in values.iter().enumerate() {
                let val = lower_expr(&val_expr.node, ctx)?;
                let offset = (i * 8) as i32;
                ctx.builder.ins().store(flags, val, ptr, offset);
            }

            Ok(ptr)
        }

        // ── FieldAccess: Fio 5 — load ptr + field_index * 8 ──
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
            let clif_ty = ty_to_clif(&expr.ty);
            Ok(ctx.builder.ins().load(clif_ty, flags, ptr, offset))
        }

        // ── IndexAccess: Fio 5 — load ptr + element_index * 8 ──
        TypedExprKind::IndexAccess {
            expr: inner,
            element_index,
            ..
        } => {
            let ptr = lower_expr(&inner.node, ctx)?;
            let flags = MemFlagsData::new();
            let offset = (*element_index as i32) * 8;
            let clif_ty = ty_to_clif(&expr.ty);
            Ok(ctx.builder.ins().load(clif_ty, flags, ptr, offset))
        }

        // ── Let: define variável ──
        TypedExprKind::Let { name, value } => {
            // Se o value é um Lambda com captures, registrar no closure_captures
            // para o call site poder alocar o CaptureBox.
            if let TypedExprKind::Lambda { captures, .. } = &value.node.kind
                && !captures.is_empty()
            {
                ctx.closure_captures.insert(name.clone(), captures.clone());
            }
            let val = lower_expr(&value.node, ctx)?;
            let clif_ty = ty_to_clif(&value.node.ty);
            let var = ctx.new_var(name, clif_ty);
            ctx.builder.def_var(var, val);
            // Let retorna Unit.
            Ok(ctx.builder.ins().iconst(I64, 0))
        }

        // ── VariantQual: Boolean::True = 1, Boolean::False = 0 ──
        // Para outros enums (Fase 5): VariantQual unitária → kata_rt_store_sum_result(tag, 0).
        TypedExprKind::VariantQual {
            enum_name,
            variant,
            tag,
        } => {
            if enum_name == "Boolean" {
                let val = if variant == "True" { 1 } else { 0 };
                Ok(ctx.builder.ins().iconst(I64, val))
            } else {
                // Variante unitária de enum do usuário: box com tag, payload = 0.
                let tag_val = ctx.builder.ins().iconst(I64, *tag as i64);
                let payload_val = ctx.builder.ins().iconst(I64, 0);
                // Arena baseada em EscapeTarget (Pré-11).
                let arena_handle = match expr.escape {
                    EscapeTarget::Local => ctx
                        .fiber_arena
                        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
                    EscapeTarget::Caller | EscapeTarget::Ancestor(_) => ctx
                        .caller_arena
                        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
                };
                let func_ref = ctx
                    .ffi_refs
                    .get("kata_rt_store_sum_result")
                    .ok_or_else(|| {
                        super::CodegenError::FfiSymbolNotFound("kata_rt_store_sum_result".into())
                    })?;
                let call_inst = ctx
                    .builder
                    .ins()
                    .call(*func_ref, &[tag_val, payload_val, arena_handle]);
                Ok(ctx.builder.inst_results(call_inst)[0])
            }
        }

        // ── VariantConstruct: Fase 5 — Sum com payload ──
        // Result::Ok 42 → kata_rt_store_sum_result(tag, payload) → box_ptr
        TypedExprKind::VariantConstruct {
            enum_name: _,
            variant: _,
            payload,
            tag,
        } => {
            // Lowera o payload.
            let payload_val = lower_expr(&payload.node, ctx)?;

            // Bitcast F64→I64 se necessário: store_sum_result espera I64
            // para o payload, mas Float lowera como F64.
            let payload_val = {
                let payload_ty = ctx.builder.func.dfg.value_type(payload_val);
                if payload_ty == cranelift_codegen::ir::types::F64 {
                    ctx.builder
                        .ins()
                        .bitcast(I64, MemFlagsData::new(), payload_val)
                } else {
                    payload_val
                }
            };

            // Tag = índice da variante (embutido no TypedExpr pelo typeck).
            let tag_val = ctx.builder.ins().iconst(I64, *tag as i64);

            // Arena baseada em EscapeTarget (Pré-11).
            let arena_handle = match expr.escape {
                EscapeTarget::Local => ctx
                    .fiber_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
                EscapeTarget::Caller | EscapeTarget::Ancestor(_) => ctx
                    .caller_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
            };

            // Chama kata_rt_store_sum_result(tag, payload, arena_handle) → box_ptr.
            let func_ref = ctx
                .ffi_refs
                .get("kata_rt_store_sum_result")
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_store_sum_result".into())
                })?;
            let call_inst = ctx
                .builder
                .ins()
                .call(*func_ref, &[tag_val, payload_val, arena_handle]);
            Ok(ctx.builder.inst_results(call_inst)[0])
        }

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
            // Se há captures, o primeiro param é box_ptr (I64).
            let mut sig = Signature::new(CallConv::Tail);
            if !captures.is_empty() {
                sig.params.push(AbiParam::new(I64)); // box_ptr
            }
            for pt in param_types {
                sig.params.push(AbiParam::new(ty_to_clif(pt)));
            }
            sig.returns.push(AbiParam::new(ty_to_clif(ret_ty)));
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
                func_id,
                ctx.module,
                ctx.ffi_ids,
                ctx.kata_ids,
                ctx.string_table,
            )?;

            // Retorna o function pointer como valor.
            // declare_func_in_func retorna FuncRef (para call direto);
            // para call_indirect precisamos de um GlobalValue apontando
            // para o símbolo da função.
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
            Ok(func_ptr)
        }

        // ── Match: pattern matching com branch chain ──
        TypedExprKind::Match { scrutinee, arms } => lower_match(scrutinee, arms, ctx),

        // ── Fase 10: ActionCall — scheduler (entry) ou call direto (dentro de Action) ──
        TypedExprKind::ActionCall {
            callee,
            args,
            caller_arena: _,
            ffi_symbol,
        } => lower_action_call(expr, callee, args, ffi_symbol, ctx),
        // ── Fio 3: Var — mesmo codegen que Let ──
        TypedExprKind::Var { name, value } => {
            let val = lower_expr(&value.node, ctx)?;
            let clif_ty = ty_to_clif(&value.node.ty);
            let var = ctx.new_var(name, clif_ty);
            ctx.builder.def_var(var, val);
            // Var retorna Unit (como Let).
            Ok(ctx.builder.ins().iconst(I64, 0))
        }

        // ── Fio 3: Reassign — def_var com novo valor (variável já existe) ──
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

        // ── Fio 3 Fase 2: return — jump para epilogue_block ──
        // ── Fio 3 Fase 4: loop, break, continue ──
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

        // ── Fio 8 Fase 6: ListLit — constrói Cons chain de trás para frente ──
        TypedExprKind::ListLit { elements } => {
            let arena_handle = match expr.escape {
                EscapeTarget::Local => ctx
                    .fiber_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
                EscapeTarget::Caller | EscapeTarget::Ancestor(_) => ctx
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
            Ok(acc)
        }

        // ── Fio 8 Fase 6: ArrayLit — aloca header+data, set cada elemento ──
        TypedExprKind::ArrayLit { elements } => {
            let n = elements.len() as i64;
            let arena_handle = match expr.escape {
                EscapeTarget::Local => ctx
                    .fiber_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
                EscapeTarget::Caller | EscapeTarget::Ancestor(_) => ctx
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
            Ok(ptr)
        }

        // ── Fio 8 Fase 6: RangeLit — aloca 3 words, store start/step/end ──
        TypedExprKind::RangeLit {
            start,
            step,
            end,
            inclusive: _,
            elem_ty: _,
        } => {
            let arena_handle = match expr.escape {
                EscapeTarget::Local => ctx
                    .fiber_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
                EscapeTarget::Caller | EscapeTarget::Ancestor(_) => ctx
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
            Ok(ptr)
        }

        // ── Fio 8 Fase 6: ForIn — loop inlined por tipo concreto ──
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
                    let next = ctx.builder.ins().iadd(current, step_val);
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

            Ok(unit)
        }

        // ── Fio 8 Fase 6: In (membership) — dispatch por tipo concreto ──
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
                    Ok(ctx.builder.inst_results(call)[0])
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
                    Ok(ctx.builder.inst_results(call)[0])
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
                    Ok(ctx.builder.ins().uextend(I64, result_i8))
                }
                _ => Err(super::CodegenError::UnsupportedNode(format!(
                    "In sobre tipo não-coleção: {coll_ty:?}"
                ))),
            }
        }
    }
}
