//! Lowering de `fork!` e `spawn!` — spawn de fiber e spawn de processo OS.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{GlobalValueData, InstBuilder, Value};
use cranelift_module::Module;
use kata_core::ty::Ty;
use kata_inference::{TypedExpr, TypedExprKind};

use super::super::LowerCtx;
use super::get_ffi;

/// Procura uma Action em `ctx.kata_ids` por `(name, param_types)`, com
/// fallback para nome apenas. Retorna o `FuncId` e a chave encontrada.
///
/// Usado por `lower_fork` e `lower_spawn` para obter o function pointer
/// da Action via `GlobalValue::Symbol`.
fn lookup_action_fn_ptr(
    ctx: &mut LowerCtx,
    action_name: &str,
    args: &kata_ast::Spanned<TypedExpr>,
) -> Result<Value, super::super::CodegenError> {
    let param_types: Vec<Ty> = match &args.node.kind {
        TypedExprKind::Unit => Vec::new(),
        TypedExprKind::Tuple { elements } => elements.iter().map(|e| e.node.ty.clone()).collect(),
        _ => vec![args.node.ty.clone()],
    };

    // Procurar a Action em kata_ids por (name, param_types).
    let mut found_key: Option<super::super::module::FuncKey> = None;
    for key in ctx.kata_ids.keys() {
        if key.0 == action_name && key.1 == param_types {
            found_key = Some(key.clone());
            break;
        }
    }
    // Fallback: se não encontrou com param_types exatos, procurar só por nome.
    if found_key.is_none() {
        for key in ctx.kata_ids.keys() {
            if key.0 == action_name {
                found_key = Some(key.clone());
                break;
            }
        }
    }

    let key = found_key.ok_or_else(|| {
        super::super::CodegenError::UnsupportedNode { node: format!(
            "Action `{action_name}` não encontrada em kata_ids"
        ) }
    })?;

    let callee_fid = ctx.kata_ids.get(&key).copied().ok_or_else(|| {
        super::super::CodegenError::UnsupportedNode { node: format!(
            "FuncId para Action `{action_name}` não encontrado"
        ) }
    })?;

    // Converte FuncId → fn_ptr (Value) via GlobalValue::Symbol.
    func_id_to_fn_ptr(ctx, callee_fid)
}

/// Converte um `FuncId` em um `Value` (fn_ptr) via `GlobalValue::Symbol`.
///
/// Pattern compartilhado por fork, spawn e o broker IPC.
pub(crate) fn func_id_to_fn_ptr(
    ctx: &mut LowerCtx,
    fid: cranelift_module::FuncId,
) -> Result<Value, super::super::CodegenError> {
    let func_ref = ctx.module.declare_func_in_func(fid, ctx.builder.func);
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
    Ok(ctx
        .builder
        .ins()
        .global_value(ctx.module.target_config().pointer_type(), func_gv))
}

/// `fork!(action, args)` — spawn de fiber.
///
/// Para fork direto (`action_name` != "__indirect_fork"), obtém o function
/// pointer da Action via `GlobalValue::Symbol` (mesmo mecanismo de
/// `lower_action_call` em scheduler_mode).
///
/// Para fork indireto (`action_name` == "__indirect_fork"), lowera
/// `action_expr` para obter o fn_ptr em runtime — a expressão avalia para
/// o fn_ptr da Action (via Ident com Ty::Action no codegen).
///
/// Lowera os args (tupla → args_ptr), e chama
/// `kata_rt_spawn(fn_ptr, caller_arena, args_ptr)`.
///
/// Retorna Unit — fork é fire-and-forget (structured concurrency garante
/// que o parent espera os filhos).
pub(crate) fn lower_fork(
    expr: &TypedExpr,
    action_name: &str,
    action_expr: &kata_ast::Spanned<TypedExpr>,
    args: &kata_ast::Spanned<TypedExpr>,
    ctx: &mut LowerCtx,
) -> Result<Value, super::super::CodegenError> {
    // 1. Lowerar args (tupla) → args_ptr.
    let args_ptr = super::super::expr::lower_expr(&args.node, ctx)?;

    // 2. Obter fn_ptr:
    //    - Fork direto: lookup em kata_ids por action_name, GlobalValue::Symbol.
    //    - Fork indireto: lower action_expr → runtime fn_ptr value.
    let fn_ptr = if action_name == "__indirect_fork" {
        // Indirect fork — lower action_expr to get fn_ptr at runtime.
        super::super::expr::lower_expr(&action_expr.node, ctx)?
    } else {
        // Direct fork — lookup by action_name in kata_ids.
        lookup_action_fn_ptr(ctx, action_name, args)?
    };

    // 5. Determinar caller_arena (onde os args vivem — EscapeTarget do expr).
    let caller_arena_val = crate::lowering::escape_arena::arena_handle_for_escape(expr.escape, ctx);

    // 6. kata_rt_spawn(fn_ptr, caller_arena, args_ptr) → fiber_id
    let spawn_ref = get_ffi(ctx, "kata_rt_spawn")?;
    ctx.builder
        .ins()
        .call(spawn_ref, &[fn_ptr, caller_arena_val, args_ptr]);

    // Fork retorna Unit.
    Ok(ctx.builder.ins().iconst(I64, 0))
}

/// Lowera `TypedExprKind::Spawn` — spawn de processo OS via fork.
///
/// Fire-and-forget como `fork!` — não retorna valor (Unit). O child herda
/// a arena via COW, executa a Action, e termina. A comunicação entre
/// parent e child é exclusivamente por canais (passados como args).
///
/// Diferença de fork!: `fork!` cria fiber no mesmo processo (via
/// `kata_rt_spawn`), `spawn!` cria processo OS separado (via
/// `kata_rt_spawn_process` que faz fork).
pub(crate) fn lower_spawn(
    _expr: &TypedExpr,
    action_name: &str,
    action_expr: &kata_ast::Spanned<TypedExpr>,
    args: &kata_ast::Spanned<TypedExpr>,
    ctx: &mut LowerCtx,
) -> Result<Value, super::super::CodegenError> {
    // 1. Lowerar args (tupla) → args_ptr.
    let args_ptr = super::super::expr::lower_expr(&args.node, ctx)?;

    // 2. Obter fn_ptr (mesma lógica de lower_fork).
    let fn_ptr = if action_name.starts_with("__indirect") {
        super::super::expr::lower_expr(&action_expr.node, ctx)?
    } else {
        lookup_action_fn_ptr(ctx, action_name, args)?
    };

    // 3. Determinar arena — usar caller_arena (root_arena no entry point).
    //    O fork() faz COW da arena do caller, e o child executa com essa arena.
    let arena_val = ctx
        .caller_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // 4. kata_rt_spawn_process(fn_ptr, args_ptr, arena) — fork e exec.
    //    Fire-and-forget: não há pipe de resultado, não há return.
    let spawn_ref = get_ffi(ctx, "kata_rt_spawn_process")?;
    ctx.builder
        .ins()
        .call(spawn_ref, &[fn_ptr, args_ptr, arena_val]);

    // 5. Retorna Unit (fire-and-forget como fork!).
    Ok(ctx.builder.ins().iconst(I64, 0))
}
