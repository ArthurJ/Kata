//! Lowering de operações CSP (Fio 11 Fase 5) — canais e fork.
//!
//! Lowera `ChannelCreate`, `ChannelSend`, `ChannelRecv`, e `Fork` da TAST
//! para chamadas FFI do runtime. `Select` fica para a Fase 6.
//!
//! - `channel!()` / `queue!(N)` / `broadcast!()` → FFI de criação + tupla (tx, rx)
//! - `tx !> valor` → `kata_rt_channel_send(handle, value)`
//! - `rx <! nome` → `kata_rt_channel_recv(handle)` + binding no var_map
//! - `fork!(action, args)` → `kata_rt_spawn(fn_ptr, caller_arena, args_ptr)`

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{InstBuilder, MemFlagsData};
use cranelift_module::Module;
use kata_core::escape::EscapeTarget;
use kata_core::ty::Ty;
use kata_inference::{ChannelKind, TypedExpr, TypedExprKind};

use super::LowerCtx;

/// Lowera `TypedExprKind::ChannelCreate`.
///
/// Chama a FFI de criação apropriada para o `ChannelKind`, depois constrói
/// uma tupla `(sender_handle, receiver_handle)` alocada na arena.
///
/// - Rendezvous/Buffered: sender e receiver são o **mesmo** handle.
/// - Broadcast: sender é o handle retornado por `broadcast_create`;
///   receiver é criado via `broadcast_receiver_create(arena, handle)`.
pub(crate) fn lower_channel_create(
    _expr: &TypedExpr,
    kind: &ChannelKind,
    _elem_ty: &Ty,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    // Arena onde o canal é alocado. Canais fluem descendente na árvore
    // de fibers — o fiber criador é always-last. Usar caller_arena se
    // disponível (canal sobrevive à destruição do fiber), senão fiber_arena.
    let arena = ctx
        .caller_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // 1. Chamar a FFI de criação conforme o kind.
    let handle = match kind {
        ChannelKind::Rendezvous => {
            let fref = ctx
                .ffi_refs
                .get("kata_rt_channel_create")
                .copied()
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_channel_create".into())
                })?;
            let inst = ctx.builder.ins().call(fref, &[arena]);
            ctx.builder.inst_results(inst)[0]
        }
        ChannelKind::Buffered(cap) => {
            let fref = ctx
                .ffi_refs
                .get("kata_rt_queue_create")
                .copied()
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_queue_create".into())
                })?;
            let cap_val = ctx.builder.ins().iconst(I64, *cap);
            let inst = ctx.builder.ins().call(fref, &[arena, cap_val]);
            ctx.builder.inst_results(inst)[0]
        }
        ChannelKind::Broadcast => {
            let fref = ctx
                .ffi_refs
                .get("kata_rt_broadcast_create")
                .copied()
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_broadcast_create".into())
                })?;
            let inst = ctx.builder.ins().call(fref, &[arena]);
            ctx.builder.inst_results(inst)[0]
        }
    };

    // 2. Para broadcast, criar o receiver via broadcast_receiver_create.
    let rx_handle = match kind {
        ChannelKind::Broadcast => {
            let fref = ctx
                .ffi_refs
                .get("kata_rt_broadcast_receiver_create")
                .copied()
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound(
                        "kata_rt_broadcast_receiver_create".into(),
                    )
                })?;
            let inst = ctx.builder.ins().call(fref, &[arena, handle]);
            ctx.builder.inst_results(inst)[0]
        }
        // Rendezvous/Buffered: sender e receiver são o mesmo handle.
        _ => handle,
    };

    // 3. Alocar tupla (handle, rx_handle) na arena — 16 bytes.
    let size = ctx.builder.ins().iconst(I64, 16);
    let alloc_fref = ctx
        .ffi_refs
        .get("kata_rt_arena_alloc")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_arena_alloc".into()))?;
    let alloc_inst = ctx.builder.ins().call(alloc_fref, &[arena, size]);
    let ptr = ctx.builder.inst_results(alloc_inst)[0];

    // 4. Store dos dois handles na tupla.
    let flags = MemFlagsData::new();
    ctx.builder.ins().store(flags, handle, ptr, 0);
    ctx.builder.ins().store(flags, rx_handle, ptr, 8);

    Ok(ptr)
}

/// Lowera `TypedExprKind::ChannelSend` (`tx !> valor`).
///
/// Chama `kata_rt_channel_send(handle, value)` e retorna Unit.
pub(crate) fn lower_channel_send(
    channel: &kata_ast::Spanned<TypedExpr>,
    value: &kata_ast::Spanned<TypedExpr>,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let handle = super::expr::lower_expr(&channel.node, ctx)?;
    let val = super::expr::lower_expr(&value.node, ctx)?;

    let fref = ctx
        .ffi_refs
        .get("kata_rt_channel_send")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_channel_send".into()))?;
    ctx.builder.ins().call(fref, &[handle, val]);

    // ChannelSend retorna Unit.
    Ok(ctx.builder.ins().iconst(I64, 0))
}

/// Lowera `TypedExprKind::ChannelRecv` (`rx <! nome`).
///
/// Chama `kata_rt_channel_recv(handle)`, cria binding `bind_name` no
/// `var_map`, e retorna o valor recebido.
pub(crate) fn lower_channel_recv(
    channel: &kata_ast::Spanned<TypedExpr>,
    bind_name: &str,
    recv_ty: &Ty,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let handle = super::expr::lower_expr(&channel.node, ctx)?;

    let fref = ctx
        .ffi_refs
        .get("kata_rt_channel_recv")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_channel_recv".into()))?;
    let inst = ctx.builder.ins().call(fref, &[handle]);
    let val = ctx.builder.inst_results(inst)[0];

    // Criar binding no var_map (igual ao Let lowering).
    let clif_ty = crate::ffi_sigs::ty_to_clif(recv_ty);
    let var = ctx.new_var(bind_name, clif_ty);
    ctx.builder.def_var(var, val);

    Ok(val)
}

/// Lowera `TypedExprKind::Fork` (`fork!(action, args)`).
///
/// Obtém o function pointer da Action via `GlobalValue::Symbol` (mesmo
/// mecanismo de `lower_action_call` em scheduler_mode), lowera os args
/// (tupla → args_ptr), e chama `kata_rt_spawn(fn_ptr, caller_arena, args_ptr)`.
///
/// Retorna Unit — fork é fire-and-forget (structured concurrency garante
/// que o parent espera os filhos).
pub(crate) fn lower_fork(
    expr: &TypedExpr,
    action_name: &str,
    args: &kata_ast::Spanned<TypedExpr>,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    // 1. Lowerar args (tupla) → args_ptr.
    let args_ptr = super::expr::lower_expr(&args.node, ctx)?;

    // 2. Extrair param_types do args (tupla tipada) para lookup no kata_ids.
    let param_types: Vec<Ty> = match &args.node.kind {
        TypedExprKind::Unit => Vec::new(),
        TypedExprKind::Tuple { elements } => elements.iter().map(|e| e.node.ty.clone()).collect(),
        _ => vec![args.node.ty.clone()],
    };

    // 3. Procurar a Action em kata_ids por (name, param_types).
    //    A inference do fork! não faz dispatch — só verifica que a Action
    //    existe. Iteramos kata_ids buscando key.0 == action_name e
    //    key.1 == param_types. Se múltiplas matcham (overloads), pegamos
    //    a primeira — a inference já validou que é uma Action válida.
    let mut found_key: Option<super::module::FuncKey> = None;
    for key in ctx.kata_ids.keys() {
        if key.0 == action_name && key.1 == param_types {
            found_key = Some(key.clone());
            break;
        }
    }
    // Fallback: se não encontrou com param_types exatos, procurar só por nome.
    // Pode acontecer se a Action tem params genéricos que foram monomorfizados.
    if found_key.is_none() {
        for key in ctx.kata_ids.keys() {
            if key.0 == action_name {
                found_key = Some(key.clone());
                break;
            }
        }
    }

    let key = found_key.ok_or_else(|| {
        super::CodegenError::UnsupportedNode(format!(
            "fork!: Action `{action_name}` não encontrada em kata_ids"
        ))
    })?;

    // 4. Obter fn_ptr via GlobalValue::Symbol (igual lower_action_call scheduler_mode).
    let callee_fid = ctx.kata_ids.get(&key).copied().ok_or_else(|| {
        super::CodegenError::UnsupportedNode(format!(
            "fork!: FuncId para Action `{action_name}` não encontrado"
        ))
    })?;
    let func_ref = ctx
        .module
        .declare_func_in_func(callee_fid, ctx.builder.func);
    let ext_func_name = ctx.builder.func.dfg.ext_funcs[func_ref].name.clone();
    let func_gv =
        ctx.builder
            .func
            .create_global_value(cranelift_codegen::ir::GlobalValueData::Symbol {
                name: ext_func_name,
                offset: 0.into(),
                colocated: true,
                tls: false,
            });
    let fn_ptr = ctx
        .builder
        .ins()
        .global_value(ctx.module.target_config().pointer_type(), func_gv);

    // 5. Determinar caller_arena (onde os args vivem — EscapeTarget do expr).
    let caller_arena_val = match expr.escape {
        EscapeTarget::Local => ctx
            .fiber_arena
            .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
        EscapeTarget::Caller | EscapeTarget::Ancestor(_) => ctx
            .caller_arena
            .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0)),
    };

    // 6. kata_rt_spawn(fn_ptr, caller_arena, args_ptr) → fiber_id
    let spawn_ref = ctx
        .ffi_refs
        .get("kata_rt_spawn")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_spawn".into()))?;
    ctx.builder
        .ins()
        .call(spawn_ref, &[fn_ptr, caller_arena_val, args_ptr]);

    // Fork retorna Unit.
    Ok(ctx.builder.ins().iconst(I64, 0))
}
