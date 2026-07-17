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

        // ── Fio 8: Coleções — lowering implementado na Fase 6 ──
        TypedExprKind::ListLit { .. }
        | TypedExprKind::ArrayLit { .. }
        | TypedExprKind::RangeLit { .. }
        | TypedExprKind::ForIn { .. }
        | TypedExprKind::In { .. } => Err(super::CodegenError::UnsupportedNode(format!(
            "{:?} — lowering de coleções é Fase 6",
            expr.kind
        ))),
    }
}
