//! Lowera uma expressão TAST → valor CLIF.
//!
//! Dispatch central — cada variante de `TypedExprKind` é lowerada aqui.
//! Funções que não são do tipo expressão (module, match, clause, pattern)
//! vivem em submódulos irmãos.

use cranelift_codegen::ir::types::{F64, I64};
use cranelift_codegen::ir::{AbiParam, GlobalValueData, InstBuilder, MemFlagsData, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_module::{Linkage, Module};
use kata_core::ty::{PrimTy, Ty};
use kata_inference::{TypedExpr, TypedExprKind};

use super::_match::lower_match;
use super::LowerCtx;
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
            if let Some(&func_id) = ctx.kata_ids.get(name) {
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
            Err(super::CodegenError::UnsupportedNode(format!(
                "unbound ident: {name}"
            )))
        }

        // ── Closure: call FFI, call direto (Kata), ou call_indirect ──
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => {
            // Lowera os argumentos.
            let mut arg_values = Vec::with_capacity(args.len());
            for arg in args {
                let val = lower_expr(&arg.node, ctx)?;
                arg_values.push(val);
            }

            if let Some(sym_name) = ffi_symbol {
                // Call FFI direto — FFI nunca é tail call (CallConv::SystemV).
                let func_ref = ctx
                    .ffi_refs
                    .get(sym_name)
                    .ok_or_else(|| super::CodegenError::FfiSymbolNotFound(sym_name.clone()))?;
                let call_inst = ctx.builder.ins().call(*func_ref, &arg_values);
                Ok(ctx.builder.inst_results(call_inst)[0])
            } else {
                // ffi_symbol = None: função Kata nomeada ou lambda como valor.
                // Tenta Kata function call direto primeiro.
                if let TypedExprKind::Ident { name } = &callee.node.kind {
                    if let Some(&func_ref) = ctx.kata_refs.get(name) {
                        // Call direto para função Kata nomeada.
                        if expr.tail_pos && !ctx.no_tail_calls {
                            // Tail call: emite return_call (TCO via Cranelift).
                            ctx.builder.ins().return_call(func_ref, &arg_values);
                            ctx.emitted_tail_call = true;
                            // return_call é terminador — não pode adicionar instruções depois.
                            // Criar block dummy unreachable para satisfazer o builder:
                            // todo block precisa de um terminador, mesmo que inalcançável.
                            let dummy = ctx.builder.create_block();
                            ctx.builder.switch_to_block(dummy);
                            ctx.builder.seal_block(dummy);
                            let val = ctx.builder.ins().iconst(I64, 0);
                            ctx.builder.ins().return_(&[val]);
                            return Ok(val);
                        }
                        let call_inst = ctx.builder.ins().call(func_ref, &arg_values);
                        return Ok(ctx.builder.inst_results(call_inst)[0]);
                    }
                    // Ident não está no kata_refs: pode ser variável com
                    // Ty::Function (lambda como valor) — call_indirect.
                    if let Some(var) = ctx.var_map.get(name) {
                        let func_ptr = ctx.builder.use_var(*var);

                        // Se há captures registradas para esta closure,
                        // alocar CaptureBox e prefixar box_ptr nos args.
                        let caps = ctx.closure_captures.get(name).cloned();
                        let mut call_args = Vec::new();
                        if let Some(ref captures) = caps {
                            if !captures.is_empty() {
                                let box_ptr = alloc_capture_box(func_ptr, captures, ctx)?;
                                call_args.push(box_ptr);
                            }
                        }
                        call_args.extend(arg_values.iter().copied());

                        // Constrói a assinatura para call_indirect.
                        // O tipo do callee é Ty::Function(params, ret).
                        let callee_ty = &callee.node.ty;
                        if let Ty::Function(param_types, ret_ty) = callee_ty {
                            let mut sig = Signature::new(CallConv::Tail);
                            // Se há captures, box_ptr é o primeiro param da sig.
                            if caps.as_ref().is_some_and(|c| !c.is_empty()) {
                                sig.params.push(AbiParam::new(I64));
                            }
                            for pt in param_types {
                                sig.params.push(AbiParam::new(ty_to_clif(pt)));
                            }
                            sig.returns.push(AbiParam::new(ty_to_clif(ret_ty)));
                            let sig_ref = ctx.builder.func.import_signature(sig);
                            if expr.tail_pos && !ctx.no_tail_calls {
                                // Tail call indireto: return_call_indirect.
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
                            return Ok(ctx.builder.inst_results(call_inst)[0]);
                        }
                    }
                }
                Err(super::CodegenError::UnsupportedNode(format!(
                    "Closure sem ffi_symbol e callee não-Ident: {:?}",
                    callee.node.kind
                )))
            }
        }

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
            // Escolha de arena baseada em tail_pos:
            // - tail_pos = true (retorno) → caller_arena (sobrevive à destruição da local)
            // - tail_pos = false (computação local) → local_arena (liberada no epílogo)
            let handle = if expr.tail_pos {
                // tail_pos = true: usar caller_arena. Se não há caller_arena
                // (função pura), usa arena global (handle 0).
                ctx.caller_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0))
            } else {
                // tail_pos = false: usar fiber_arena. Se não há fiber_arena
                // (entry point ou função pura), usa arena global (handle 0).
                // Entry point: tudo é tail_pos, este branch não é atingido.
                // Função pura: não há epílogo que destrua, arena global é segura.
                ctx.fiber_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0))
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

        // ── Let: define variável ──
        TypedExprKind::Let { name, value } => {
            // Se o value é um Lambda com captures, registrar no closure_captures
            // para o call site poder alocar o CaptureBox.
            if let TypedExprKind::Lambda { captures, .. } = &value.node.kind {
                if !captures.is_empty() {
                    ctx.closure_captures.insert(name.clone(), captures.clone());
                }
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
                let func_ref = ctx
                    .ffi_refs
                    .get("kata_rt_store_sum_result")
                    .ok_or_else(|| {
                        super::CodegenError::FfiSymbolNotFound("kata_rt_store_sum_result".into())
                    })?;
                let call_inst = ctx.builder.ins().call(*func_ref, &[tag_val, payload_val]);
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

            // Tag = índice da variante (embutido no TypedExpr pelo typeck).
            let tag_val = ctx.builder.ins().iconst(I64, *tag as i64);

            // Chama kata_rt_store_sum_result(tag, payload) → box_ptr.
            let func_ref = ctx
                .ffi_refs
                .get("kata_rt_store_sum_result")
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_store_sum_result".into())
                })?;
            let call_inst = ctx.builder.ins().call(*func_ref, &[tag_val, payload_val]);
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
            super::module::define_function_body(
                &name,
                param_types,
                ret_ty,
                clauses,
                captures,
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
        } => {
            // Lowera os argumentos (tupla) → args_ptr (ponteiro para a tupla na arena).
            let args_ptr = lower_expr(&args.node, ctx)?;

            // Despacha: se tem ffi_symbol, é Action builtin FFI (ex: echo, panic).
            // Builtins NÃO passam pelo scheduler — são calls FFI diretos.
            if let Some(sym_name) = ffi_symbol {
                // Extrai elementos da tupla para passar como args individuais ao FFI.
                let mut ffi_args = Vec::new();
                match &args.node.kind {
                    TypedExprKind::Unit => {}
                    TypedExprKind::Tuple { elements } => {
                        let flags = MemFlagsData::new();
                        for (i, _elem) in elements.iter().enumerate() {
                            let offset = (i * 8) as i32;
                            let val = ctx.builder.ins().load(I64, flags, args_ptr, offset);
                            ffi_args.push(val);
                        }
                    }
                    _ => {
                        ffi_args.push(args_ptr);
                    }
                }
                let func_ref = ctx
                    .ffi_refs
                    .get(sym_name)
                    .ok_or_else(|| super::CodegenError::FfiSymbolNotFound(sym_name.clone()))?;
                let call_inst = ctx.builder.ins().call(*func_ref, &ffi_args);
                if let Some(ret) = ctx.builder.inst_results(call_inst).first() {
                    Ok(*ret)
                } else {
                    Ok(ctx.builder.ins().iconst(I64, 0))
                }
            } else if let Some(&func_ref) = ctx.kata_refs.get(callee) {
                // Action definida pelo usuário.
                // ABI uniforme: (fiber_arena, caller_arena, args_ptr) -> i64.

                // caller_arena decidido por tail_pos:
                // - tail_pos = true: ctx.caller_arena (sobrevive à destruição do fiber)
                // - tail_pos = false: ctx.fiber_arena (arena local do fiber)
                let caller_arena_val = if expr.tail_pos {
                    ctx.caller_arena
                        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0))
                } else {
                    ctx.fiber_arena
                        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0))
                };

                if ctx.scheduler_mode {
                    // Entry point: spawn + run (scheduler cria fiber + arena).
                    // 1. Obter fn_ptr via GlobalValue::Symbol.
                    let callee_fid = ctx.kata_ids.get(callee).ok_or_else(|| {
                        super::CodegenError::UnsupportedNode(format!(
                            "ActionCall: callee `{callee}` não encontrado em kata_ids"
                        ))
                    })?;
                    let func_ref2 = ctx
                        .module
                        .declare_func_in_func(*callee_fid, ctx.builder.func);
                    let ext_func_name = ctx.builder.func.dfg.ext_funcs[func_ref2].name.clone();
                    let func_gv = ctx.builder.func.create_global_value(
                        cranelift_codegen::ir::GlobalValueData::Symbol {
                            name: ext_func_name,
                            offset: 0.into(),
                            colocated: true,
                            tls: false,
                        },
                    );
                    let fn_ptr = ctx
                        .builder
                        .ins()
                        .global_value(ctx.module.target_config().pointer_type(), func_gv);

                    // 2. spawn(fn_ptr, caller_arena, args_ptr) → fiber_id
                    let spawn_ref =
                        ctx.ffi_refs.get("kata_rt_spawn").copied().ok_or_else(|| {
                            super::CodegenError::FfiSymbolNotFound("kata_rt_spawn".into())
                        })?;
                    let spawn_inst = ctx
                        .builder
                        .ins()
                        .call(spawn_ref, &[fn_ptr, caller_arena_val, args_ptr]);
                    let _fiber_id = ctx.builder.inst_results(spawn_inst)[0];

                    // 3. run() → result (i64)
                    let run_ref = ctx.ffi_refs.get("kata_rt_run").copied().ok_or_else(|| {
                        super::CodegenError::FfiSymbolNotFound("kata_rt_run".into())
                    })?;
                    let run_inst = ctx.builder.ins().call(run_ref, &[]);
                    let result = ctx.builder.inst_results(run_inst)[0];

                    // 4. Se ret_ty == Float: bitcast(F64 ← I64)
                    if expr.ty == Ty::float() {
                        Ok(ctx.builder.ins().bitcast(F64, MemFlagsData::new(), result))
                    } else {
                        Ok(result)
                    }
                } else {
                    // Dentro de Action: call direto (mesmo fiber, mesmo stack).
                    // arg_values = [fiber_arena, caller_arena, args_ptr]
                    let fiber_arena_val = ctx
                        .fiber_arena
                        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
                    let arg_values = [fiber_arena_val, caller_arena_val, args_ptr];
                    let call_inst = ctx.builder.ins().call(func_ref, &arg_values);
                    let result = ctx.builder.inst_results(call_inst)[0];

                    // Se ret_ty == Float: bitcast(F64 ← I64)
                    if expr.ty == Ty::float() {
                        Ok(ctx.builder.ins().bitcast(F64, MemFlagsData::new(), result))
                    } else {
                        Ok(result)
                    }
                }
            } else {
                Err(super::CodegenError::UnsupportedNode(format!(
                    "ActionCall: callee `{callee}` não encontrado"
                )))
            }
        }
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
        TypedExprKind::Return(inner) => {
            let val = lower_expr(&inner.node, ctx)?;
            let epilogue = ctx.epilogue_block.expect("return fora de Action");
            ctx.builder
                .ins()
                .jump(epilogue, &[cranelift_codegen::ir::BlockArg::Value(val)]);
            // Após jump (terminador), o block está fechado. Não pode adicionar
            // instruções. Retornamos `val` — o caller do loop em define_kata_action
            // detecta Return e break, então este valor é unreachable.
            Ok(val)
        }

        // ── Fio 3 Fase 4: loop, break, continue ──
        TypedExprKind::Loop { body } => {
            // Cria 3 blocks: loop_block (início do body), continue_block
            // (target de continue), break_block (target de break / saída).
            let loop_block = ctx.builder.create_block();
            let continue_block = ctx.builder.create_block();
            let break_block = ctx.builder.create_block();

            // Salva e configura loop blocks no ctx.
            let prev_break = ctx.loop_break_block;
            let prev_continue = ctx.loop_continue_block;
            ctx.loop_break_block = Some(break_block);
            ctx.loop_continue_block = Some(continue_block);

            // Entra no loop (predecessor 1 de loop_block).
            ctx.builder.ins().jump(loop_block, &[]);

            // Lowera o body no loop_block.
            ctx.builder.switch_to_block(loop_block);
            let mut hit_terminator = false;
            for expr in body {
                lower_expr(&expr.node, ctx)?;
                if matches!(
                    expr.node.kind,
                    TypedExprKind::Break | TypedExprKind::Continue | TypedExprKind::Return(_)
                ) {
                    hit_terminator = true;
                    break;
                }
            }
            // Fallthrough do body → continue_block (próxima iteração).
            if !hit_terminator {
                ctx.builder.ins().jump(continue_block, &[]);
            }

            // continue_block: jump de volta para loop_block (predecessor 2).
            ctx.builder.switch_to_block(continue_block);
            ctx.builder.ins().jump(loop_block, &[]);

            // Agora que ambos predecessores de loop_block são conhecidos
            // (entry + continue_block), podemos selar.
            ctx.builder.seal_block(loop_block);
            ctx.builder.seal_block(continue_block);

            // break_block: retorna Unit.
            ctx.builder.switch_to_block(break_block);
            ctx.builder.seal_block(break_block);
            let unit = ctx.builder.ins().iconst(I64, 0);

            // Restaura ctx.
            ctx.loop_break_block = prev_break;
            ctx.loop_continue_block = prev_continue;

            Ok(unit)
        }
        TypedExprKind::Break => {
            let break_block = ctx
                .loop_break_block
                .expect("break fora de loop (typeck deveria ter rejeitado)");
            // Cria valor Unit ANTES do jump (o jump é terminador e fecha o block).
            let unit = ctx.builder.ins().iconst(I64, 0);
            ctx.builder.ins().jump(break_block, &[]);
            // Após jump (terminador), o block está fechado. O caller detecta
            // Break e não usa o valor de retorno.
            Ok(unit)
        }
        TypedExprKind::Continue => {
            let continue_block = ctx
                .loop_continue_block
                .expect("continue fora de loop (typeck deveria ter rejeitado)");
            let unit = ctx.builder.ins().iconst(I64, 0);
            ctx.builder.ins().jump(continue_block, &[]);
            Ok(unit)
        }
    }
}

/// Aloca um CaptureBox na arena global e retorna o ponteiro.
///
/// 1. Aloca um array temporário de `n_captures * 8` bytes na arena global.
/// 2. Preenche o array com os valores das captures (lidos do var_map).
/// 3. Chama `kata_rt_alloc_arc(fn_ptr, array_ptr, n_captures)` → `box_ptr`.
///
/// O CaptureBox contém: fn_ptr (offset 0), refcount=1 (offset 8),
/// captures[0..n] (offset 16+).
fn alloc_capture_box(
    func_ptr: cranelift_codegen::ir::Value,
    captures: &[kata_inference::CaptureInfo],
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let n = captures.len() as i64;
    let flags = cranelift_codegen::ir::MemFlagsData::new();

    // 1. Aloca array temporário na arena global (handle 0).
    let arena_alloc_ref = ctx
        .ffi_refs
        .get("kata_rt_arena_alloc")
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_arena_alloc".into()))?;
    let global_arena = ctx.builder.ins().iconst(I64, 0);
    let array_size = ctx.builder.ins().iconst(I64, n * 8);
    let alloc_inst = ctx
        .builder
        .ins()
        .call(*arena_alloc_ref, &[global_arena, array_size]);
    let array_ptr = ctx.builder.inst_results(alloc_inst)[0];

    // 2. Preenche o array com os valores das captures.
    for (i, cap) in captures.iter().enumerate() {
        let cap_var = ctx.var_map.get(&cap.name).ok_or_else(|| {
            super::CodegenError::UnsupportedNode(format!(
                "capture '{}' não encontrada no var_map",
                cap.name
            ))
        })?;
        let cap_val = ctx.builder.use_var(*cap_var);
        let offset = (i * 8) as i32;
        ctx.builder.ins().store(flags, cap_val, array_ptr, offset);
    }

    // 3. Chama kata_rt_alloc_arc(fn_ptr, array_ptr, n_captures) → box_ptr.
    let alloc_arc_ref = ctx
        .ffi_refs
        .get("kata_rt_alloc_arc")
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_alloc_arc".into()))?;
    let n_val = ctx.builder.ins().iconst(I64, n);
    let arc_inst = ctx
        .builder
        .ins()
        .call(*alloc_arc_ref, &[func_ptr, array_ptr, n_val]);
    let box_ptr = ctx.builder.inst_results(arc_inst)[0];

    Ok(box_ptr)
}
