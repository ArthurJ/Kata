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
            captures: _,
            escapes: _,
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
                        // Constrói a assinatura para call_indirect.
                        // O tipo do callee é Ty::Function(params, ret).
                        let callee_ty = &callee.node.ty;
                        if let Ty::Function(param_types, ret_ty) = callee_ty {
                            let mut sig = Signature::new(CallConv::Tail);
                            for pt in param_types {
                                sig.params.push(AbiParam::new(ty_to_clif(pt)));
                            }
                            sig.returns.push(AbiParam::new(ty_to_clif(ret_ty)));
                            let sig_ref = ctx.builder.func.import_signature(sig);
                            if expr.tail_pos && !ctx.no_tail_calls {
                                // Tail call indireto: return_call_indirect.
                                ctx.builder.ins().return_call_indirect(
                                    sig_ref,
                                    func_ptr,
                                    &arg_values,
                                );
                                ctx.emitted_tail_call = true;
                                let dummy = ctx.builder.create_block();
                                ctx.builder.switch_to_block(dummy);
                                ctx.builder.seal_block(dummy);
                                let val = ctx.builder.ins().iconst(I64, 0);
                                ctx.builder.ins().return_(&[val]);
                                return Ok(val);
                            }
                            let call_inst =
                                ctx.builder
                                    .ins()
                                    .call_indirect(sig_ref, func_ptr, &arg_values);
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
                // tail_pos = false: usar local_arena. Se não há local_arena
                // (entry point ou função pura), usa arena global (handle 0).
                // Entry point: tudo é tail_pos, este branch não é atingido.
                // Função pura: não há epílogo que destrua, arena global é segura.
                ctx.local_arena
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
            let val = lower_expr(&value.node, ctx)?;
            let clif_ty = ty_to_clif(&value.node.ty);
            let var = ctx.new_var(name, clif_ty);
            ctx.builder.def_var(var, val);
            // Let retorna Unit.
            Ok(ctx.builder.ins().iconst(I64, 0))
        }

        // ── VariantQual: Boolean::True = 1, Boolean::False = 0 ──
        TypedExprKind::VariantQual { enum_name, variant } => {
            if enum_name == "Boolean" {
                let val = if variant == "True" { 1 } else { 0 };
                Ok(ctx.builder.ins().iconst(I64, val))
            } else {
                Err(super::CodegenError::UnsupportedNode(format!(
                    "VariantQual não suportado: {enum_name}::{variant}"
                )))
            }
        }

        // ── Lambda: função Cranelift separada (anon) ou referência (nomeado) ──
        TypedExprKind::Lambda {
            func_name,
            param_types,
            ret_ty,
            clauses,
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
            let mut sig = Signature::new(CallConv::Tail);
            for pt in param_types {
                sig.params.push(AbiParam::new(ty_to_clif(pt)));
            }
            sig.returns.push(AbiParam::new(ty_to_clif(ret_ty)));
            let func_id = ctx
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| super::CodegenError::Cranelift(format!("declare fn {name}: {e}")))?;

            // Compila o corpo usando o pipeline compartilhado.
            // NOTA: isto é uma simplificação — em produção, lambdas anônimos
            // deveriam ser compilados antes do entry point (como funções
            // nomeadas). Por enquanto, compilamos inline e usamos
            // finalize_definitions no jit_eval para resolver.
            super::module::define_function_body(
                &name,
                param_types,
                ret_ty,
                clauses,
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

        // ── Fio 3: ActionCall — call para Action com caller_arena handle ──
        TypedExprKind::ActionCall {
            callee,
            args,
            caller_arena: _,
            ffi_symbol,
        } => {
            // Lowera os argumentos (tupla).
            let args_val = lower_expr(&args.node, ctx)?;

            // Extrai os elementos da tupla para passar como args individuais.
            // O ABI da Action é: (caller_arena: i64, arg1, arg2, ...) -> ret_ty.
            let mut arg_values = Vec::new();

            // caller_arena handle — decide qual arena passar como caller_arena
            // do callee baseado em tail_pos:
            // - tail_pos = true: passa ctx.caller_arena (arena do caller do caller).
            //   O callee aloca retornos nessa arena, que persiste após o caller
            //   terminar. Evita UAF: o valor retorado sobrevive à destruição da
            //   local_arena do caller.
            // - tail_pos = false: passa ctx.local_arena (arena local do caller).
            //   O callee aloca retornos na arena local do caller, que é destruída
            //   no epílogo. Correto para `;` statements (valor é descartado).
            //
            // Se não há caller_arena/local_arena (entry point), usa sentinel 0
            // (arena global — handle 0 é a primeira arena criada no pool).
            let caller_arena_val = if expr.tail_pos {
                ctx.caller_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0))
            } else {
                ctx.local_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0))
            };
            arg_values.push(caller_arena_val);

            // Extrai elementos da tupla. Se é Unit (tupla vazia), não há args.
            match &args.node.kind {
                TypedExprKind::Unit => {}
                TypedExprKind::Tuple { elements } => {
                    // args_val é um ponteiro para a tupla na arena.
                    // Carrega cada elemento no offset i * 8.
                    let flags = MemFlagsData::new();
                    for (i, _elem) in elements.iter().enumerate() {
                        let offset = (i * 8) as i32;
                        let val = ctx.builder.ins().load(I64, flags, args_val, offset);
                        arg_values.push(val);
                    }
                }
                _ => {
                    // Args não-tupla (não deveria acontecer — parser sempre produz tupla).
                    arg_values.push(args_val);
                }
            }

            // Despacha: se tem ffi_symbol, é Action builtin FFI.
            // Se não, é Action definida pelo usuário (despacha via kata_refs).
            if let Some(sym_name) = ffi_symbol {
                // Action builtin FFI (ex: echo → kata_rt_print).
                // O ABI da Action builtin não tem caller_arena — é FFI direto.
                // Remove o caller_arena handle (primeiro arg).
                let ffi_args = &arg_values[1..];
                let func_ref = ctx
                    .ffi_refs
                    .get(sym_name)
                    .ok_or_else(|| super::CodegenError::FfiSymbolNotFound(sym_name.clone()))?;
                let call_inst = ctx.builder.ins().call(*func_ref, ffi_args);
                // echo retorna Unit — se não há return values, retorna 0.
                if let Some(ret) = ctx.builder.inst_results(call_inst).first() {
                    Ok(*ret)
                } else {
                    Ok(ctx.builder.ins().iconst(I64, 0))
                }
            } else if let Some(&func_ref) = ctx.kata_refs.get(callee) {
                // Action definida pelo usuário.
                let call_inst = ctx.builder.ins().call(func_ref, &arg_values);
                Ok(ctx.builder.inst_results(call_inst)[0])
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
    }
}
