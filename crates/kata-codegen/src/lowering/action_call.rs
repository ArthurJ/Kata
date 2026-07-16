//! Lowering de ActionCall — arm `ActionCall` do match `lower_expr`.
//!
//! Extraído de `expr.rs` para reduzir o tamanho do dispatch central.
//! Contém a lógica de scheduler mode (spawn+run) vs call direto (dentro de Action),
//! e FFI builtin dispatch.

use cranelift_codegen::ir::types::{F64, I64};
use cranelift_codegen::ir::{InstBuilder, MemFlagsData};
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
            // - Caller | Ancestor(_) → caller_arena (sobrevive à destruição do fiber)
            let caller_arena_val = match expr.escape {
                EscapeTarget::Local => ctx
                    .fiber_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
                EscapeTarget::Caller | EscapeTarget::Ancestor(_) => ctx
                    .caller_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
            };

            if ctx.scheduler_mode {
                // Entry point: spawn + run (scheduler cria fiber + arena).
                // 1. Obter fn_ptr via GlobalValue::Symbol.
                let callee_fid = ctx.kata_ids.get(&key).ok_or_else(|| {
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
                let run_ref = ctx
                    .ffi_refs
                    .get("kata_rt_run")
                    .copied()
                    .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_run".into()))?;
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
}