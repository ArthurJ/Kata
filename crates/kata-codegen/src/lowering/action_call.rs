//! Lowering de ActionCall — arm `ActionCall` do match `lower_expr`.
//!
//! Extraído de `expr.rs` para reduzir o tamanho do dispatch central.
//! Contém a lógica de scheduler mode (spawn+run) vs call direto (dentro de Action),
//! e FFI builtin dispatch.

use cranelift_codegen::ir::types::{F64, I64};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_module::Module;
use kata_core::escape::EscapeTarget;
use kata_core::ty::Ty;
use kata_inference::TypedExpr;

use super::LowerCtx;

/// Lowera o arm `TypedExprKind::ActionCall`.
///
/// Despacha em 3 caminhos:
/// 1. FFI builtin (echo, panic) — call FFI direto
/// 2. Action definida pelo usuário em scheduler_mode (entry point) — spawn+run
/// 3. Action definida pelo usuário dentro de Action — call direto (mesmo fiber)
pub(crate) fn lower_action_call(
    expr: &TypedExpr,
    callee: &str,
    args: &kata_ast::Spanned<TypedExpr>,
    ffi_symbol: &Option<String>,
    indirect_callee: &Option<Box<kata_ast::Spanned<TypedExpr>>>,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    // Lowera os argumentos (tupla) → args_ptr (ponteiro para a tupla na arena).
    let args_ptr = super::expr::lower_expr(&args.node, ctx)?;

    // Despacha: se tem ffi_symbol, é Action builtin FFI (ex: echo, panic).
    // Builtins NÃO passam pelo scheduler — são calls FFI diretos.
    if let Some(sym_name) = ffi_symbol {
        // Extrai elementos da tupla para passar como args individuais ao FFI.
        let mut ffi_args = Vec::new();
        match &args.node.kind {
            kata_inference::TypedExprKind::Unit => {}
            kata_inference::TypedExprKind::Tuple { elements } => {
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
        let func_ref =
            ctx.ffi_refs
                .get(sym_name)
                .ok_or_else(|| super::CodegenError::FfiSymbolNotFound {
                    symbol: sym_name.clone(),
                })?;
        let call_inst = ctx.builder.ins().call(*func_ref, &ffi_args);
        if let Some(ret) = ctx.builder.inst_results(call_inst).first() {
            Ok(*ret)
        } else {
            Ok(ctx.builder.ins().iconst(I64, 0))
        }
    } else if let Some(callee_expr) = indirect_callee {
        // NOVO: invocação indireta — fn_ptr vem da expressão (variável/param).
        // 1. Lowerar a expressão do callee → fn_ptr (i64)
        let fn_ptr = super::expr::lower_expr(&callee_expr.node, ctx)?;
        // A2: Preparar args: [rt, fiber_arena, caller_arena, args_ptr]
        //    ABI: (rt: i64, fiber_arena: i64, caller_arena: i64, args_ptr: i64) -> i64
        let rt_val = ctx.rt.unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
        let fiber_arena_val = ctx
            .fiber_arena
            .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
        let caller_arena_val = match expr.escape {
            EscapeTarget::Local => fiber_arena_val,
            EscapeTarget::Caller => ctx
                .caller_arena
                .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
            EscapeTarget::Heap => {
                // Heap escape: args devem sobreviver ao fiber — usar root_arena.
                let get_root = ctx
                    .ffi_refs
                    .get("kata_rt_get_root_arena_handle")
                    .copied()
                    .ok_or_else(|| super::CodegenError::FfiSymbolNotFound {
                        symbol: "kata_rt_get_root_arena_handle".into(),
                    })?;
                let root_inst = ctx.builder.ins().call(get_root, &[rt_val]);
                ctx.builder.inst_results(root_inst)[0]
            }
        };
        let arg_values = [rt_val, fiber_arena_val, caller_arena_val, args_ptr];
        // 3. call_indirect — assinatura Action ABI: (I64, I64, I64, I64) -> I64
        let mut sig = Signature::new(CallConv::Tail);
        sig.params.push(AbiParam::new(I64)); // rt
        sig.params.push(AbiParam::new(I64)); // fiber_arena
        sig.params.push(AbiParam::new(I64)); // caller_arena
        sig.params.push(AbiParam::new(I64)); // args_ptr
        sig.returns.push(AbiParam::new(I64)); // sempre I64
        let sig_ref = ctx.builder.func.import_signature(sig);
        let call_inst = ctx
            .builder
            .ins()
            .call_indirect(sig_ref, fn_ptr, &arg_values);
        let result = ctx.builder.inst_results(call_inst)[0];
        // Se ret_ty == Float: bitcast(F64 ← I64)
        if expr.ty == Ty::float() {
            Ok(ctx.builder.ins().bitcast(F64, MemFlagsData::new(), result))
        } else {
            Ok(result)
        }
    } else {
        // Action definida pelo usuário — lookup por chave composta.
        // Actions têm nomes únicos, mas a chave é (name, params, ret) para
        // consistência com symbol_table. Extrai params do args (tupla) e
        // ret de expr.ty.
        let param_types: Vec<Ty> = match &args.node.kind {
            kata_inference::TypedExprKind::Unit => Vec::new(),
            kata_inference::TypedExprKind::Tuple { elements } => {
                elements.iter().map(|e| e.node.ty.clone()).collect()
            }
            _ => vec![args.node.ty.clone()],
        };
        let key = (callee.to_string(), param_types, expr.ty.clone());
        if let Some(&func_ref) = ctx.kata_refs.get(&key) {
            // Action definida pelo usuário.
            // ABI uniforme: (fiber_arena, caller_arena, args_ptr) -> i64.

            // caller_arena decidido por EscapeTarget (Pré-11):
            // - Local → fiber_arena (arena local do fiber)
            // - Caller → caller_arena (sobrevive à destruição do fiber)
            let caller_arena_val =
                crate::lowering::escape_arena::arena_handle_for_escape(expr.escape, ctx);

            if ctx.scheduler_mode {
                // Entry point: spawn + run (scheduler cria fiber + arena).
                // 1. Obter fn_ptr via GlobalValue::Symbol.
                let callee_fid =
                    ctx.kata_ids
                        .get(&key)
                        .ok_or_else(|| super::CodegenError::UnsupportedNode {
                            node: format!(
                                "ActionCall: callee `{callee}` não encontrado em kata_ids"
                            ),
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

                // 2. spawn(rt, fn_ptr, caller_arena, args_ptr) → fiber_id
                let spawn_ref = ctx.ffi_refs.get("kata_rt_spawn").copied().ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound {
                        symbol: "kata_rt_spawn".into(),
                    }
                })?;
                let rt_val = ctx.rt.unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
                let spawn_inst = ctx
                    .builder
                    .ins()
                    .call(spawn_ref, &[rt_val, fn_ptr, caller_arena_val, args_ptr]);
                let _fiber_id = ctx.builder.inst_results(spawn_inst)[0];

                // 3. run(rt) → result (i64)
                let run_ref = ctx.ffi_refs.get("kata_rt_run").copied().ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound {
                        symbol: "kata_rt_run".into(),
                    }
                })?;
                let run_inst = ctx.builder.ins().call(run_ref, &[rt_val]);
                let result = ctx.builder.inst_results(run_inst)[0];

                // 4. Se ret_ty == Float: bitcast(F64 ← I64)
                if expr.ty == Ty::float() {
                    Ok(ctx.builder.ins().bitcast(F64, MemFlagsData::new(), result))
                } else {
                    Ok(result)
                }
            } else {
                // Dentro de Action: call direto (mesmo fiber, mesmo stack).
                // A2: arg_values = [rt, fiber_arena, caller_arena, args_ptr]
                let rt_val = ctx.rt.unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
                let fiber_arena_val = ctx
                    .fiber_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
                let arg_values = [rt_val, fiber_arena_val, caller_arena_val, args_ptr];
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
            Err(super::CodegenError::UnsupportedNode {
                node: format!("ActionCall: callee `{callee}` não encontrado"),
            })
        }
    }
}
