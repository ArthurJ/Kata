//! Lowering de operações de canal — create, send, recv, receiver factory.

use cranelift_codegen::ir::Value;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{InstBuilder, MemFlagsData};
use kata_core::ty::Ty;
use kata_inference::{ChannelKind, TypedExpr};

use super::super::LowerCtx;
use super::get_ffi;

/// Extrai o `type_id` do tipo de elemento do canal a partir do tipo do
/// `ChannelCreate` (`Tuple([Sender(T), Receiver(T)])`). O `elem_ty` no
/// `ChannelCreate` pode ser `Var("T0")` não-resolvido; o tipo concreto
/// está em `expr.ty`.
pub(crate) fn lookup_type_id(expr: &TypedExpr, ctx: &LowerCtx) -> i64 {
    // Tenta extrair o tipo de elemento do Sender no tipo do expr.
    let elem_ty = match &expr.ty {
        Ty::Tuple(elems) if elems.len() == 2 => {
            if let Ty::Sender(inner) = &elems[0] {
                inner.as_ref().clone()
            } else {
                Ty::Prim(kata_core::ty::PrimTy::Int)
            }
        }
        _ => Ty::Prim(kata_core::ty::PrimTy::Int),
    };

    // Procura no type_id_map. Se não encontra, usa 0 (Prim).
    ctx.type_id_map.get(&elem_ty).copied().unwrap_or(0)
}

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
    cross_process: bool,
    ctx: &mut LowerCtx,
) -> Result<Value, super::super::CodegenError> {
    // Arena onde o canal é alocado — fiber_arena do criador. Canais fluem
    // apenas descendente (pai → filho via fork!/spawn!) e structured concurrency
    // garante que o criador é always-last. bump.reset() no epílogo libera
    // o canal — O(1), sem leak. Valores ARC-managed enviados pelo canal
    // continuam na root_arena (escape analysis marca Heap no channel_send).
    let arena = ctx
        .fiber_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    let rt_val = ctx.rt.unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // 1. Chamar a FFI de criação conforme o kind e backend (in-process vs IPC).
    let handle = match kind {
        ChannelKind::Rendezvous => {
            let ffi_name = if cross_process {
                "kata_rt_ipc_channel_create"
            } else {
                "kata_rt_channel_create"
            };
            let fref = get_ffi(ctx, ffi_name)?;
            if cross_process {
                // IPC: (arena, type_id, ack_tx_handle) -> handle
                // Rendezvous IPC não usa auto-ack — ack_tx_handle = 0.
                let type_id = lookup_type_id(_expr, ctx);
                let type_id_val = ctx.builder.ins().iconst(I64, type_id);
                let ack_val = ctx.builder.ins().iconst(I64, 0);
                let inst = ctx.builder.ins().call(fref, &[arena, type_id_val, ack_val]);
                ctx.builder.inst_results(inst)[0]
            } else {
                // In-process: (arena) -> handle
                let inst = ctx.builder.ins().call(fref, &[arena]);
                ctx.builder.inst_results(inst)[0]
            }
        }
        ChannelKind::Buffered(cap) => {
            let cap_val = ctx.builder.ins().iconst(I64, *cap);
            if cross_process {
                // Buffered IPC: cria queue IPC cross-process com broker.

                // Chama kata_rt_ipc_queue_create(arena, cap, type_id) → ptr (6 handles).
                // O codegen desempacota os handles e sintetiza o broker via fork!.
                // (Implementação do broker em synthesize_ipc_broker — abaixo.)
                let type_id = lookup_type_id(_expr, ctx);
                let type_id_val = ctx.builder.ins().iconst(I64, type_id);
                let fref = get_ffi(ctx, "kata_rt_ipc_queue_create")?;
                let inst = ctx.builder.ins().call(fref, &[arena, cap_val, type_id_val]);
                let handles_ptr = ctx.builder.inst_results(inst)[0];

                // Desempacotar 6 handles: [queue_tx, queue_rx, ipc_data_tx, ipc_data_rx, ack_tx, ack_rx]
                let flags = MemFlagsData::new();
                let queue_tx = ctx.builder.ins().load(I64, flags, handles_ptr, 0);
                let queue_rx = ctx.builder.ins().load(I64, flags, handles_ptr, 8);
                let ipc_data_tx = ctx.builder.ins().load(I64, flags, handles_ptr, 16);
                let ipc_data_rx = ctx.builder.ins().load(I64, flags, handles_ptr, 24);
                let _ack_tx = ctx.builder.ins().load(I64, flags, handles_ptr, 32);
                let ack_rx = ctx.builder.ins().load(I64, flags, handles_ptr, 40);

                // Sintetizar a função JIT do broker e spawná-la via kata_rt_spawn.
                // synthesize_ipc_broker declara+define a função JIT separada e
                // retorna o FuncId. Convertemos o FuncId em fn_ptr (Value) via
                // GlobalValue::Symbol — mesmo pattern de lower_fork/lower_spawn.
                let broker_fid = super::broker::synthesize_ipc_broker(ctx)?;
                let broker_args_ptr =
                    super::broker::build_broker_args(ctx, queue_rx, ack_rx, ipc_data_tx)?;
                let broker_fn_ptr = {
                    let func_ref = ctx
                        .module
                        .declare_func_in_func(broker_fid, ctx.builder.func);
                    let ext_func_name = ctx.builder.func.dfg.ext_funcs[func_ref].name.clone();
                    let func_gv = ctx.builder.func.create_global_value(
                        cranelift_codegen::ir::GlobalValueData::Symbol {
                            name: ext_func_name,
                            offset: 0.into(),
                            colocated: true,
                            tls: false,
                        },
                    );
                    ctx.builder
                        .ins()
                        .global_value(ctx.module.target_config().pointer_type(), func_gv)
                };
                let caller_arena_val = ctx
                    .caller_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
                let spawn_ref = get_ffi(ctx, "kata_rt_spawn")?;
                ctx.builder.ins().call(
                    spawn_ref,
                    &[rt_val, broker_fn_ptr, caller_arena_val, broker_args_ptr],
                );

                // A tupla que o usuário vê é (queue_tx, ipc_data_rx).
                // Alocar tupla de 2 handles na arena (16 bytes).
                let tup_size = ctx.builder.ins().iconst(I64, 16);
                let alloc_fref = get_ffi(ctx, "kata_rt_arena_alloc")?;
                let alloc_inst = ctx
                    .builder
                    .ins()
                    .call(alloc_fref, &[rt_val, arena, tup_size]);
                let tup_ptr = ctx.builder.inst_results(alloc_inst)[0];
                ctx.builder.ins().store(flags, queue_tx, tup_ptr, 0);
                ctx.builder.ins().store(flags, ipc_data_rx, tup_ptr, 8);
                return Ok(tup_ptr);
            }
            // In-process buffered (não cross_process).
            let fref = get_ffi(ctx, "kata_rt_queue_create")?;
            let policy_val = ctx.builder.ins().iconst(I64, 0); // Block (default)
            let inst = ctx.builder.ins().call(fref, &[arena, cap_val, policy_val]);
            ctx.builder.inst_results(inst)[0]
        }
        ChannelKind::Broadcast => {
            if cross_process {
                return Err(super::super::CodegenError::UnsupportedNode { node:
                    "broadcast!() cross-process não suportado — o child não vê writes do parent após fork (COW copy)".into(),
                 });
            }
            // Broadcast in-process.
            let fref = get_ffi(ctx, "kata_rt_broadcast_create")?;
            let inst = ctx.builder.ins().call(fref, &[arena]);
            ctx.builder.inst_results(inst)[0]
        }
    };

    // 2. Para broadcast, o segundo elemento da tupla é a **ReceiverFactory**.
    //    A factory é o próprio BroadcastInner (tag 0b10) — o mesmo handle do
    //    sender. O typeck marca `bcast.1` como `ReceiverFactory::T`; em runtime
    //    é o mesmo ponteiro+tag do BroadcastInner. `rxf!()` chama
    //    `broadcast_receiver_create(arena, factory_handle)` passando este
    //    handle, que aceita tag 0b10 (ver `kata_rt_broadcast_receiver_create`).
    //    O primeiro receiver **não** é criado aqui — ele só nasce via `rxf!()`.
    let rx_handle = match kind {
        ChannelKind::Broadcast => handle,
        // Rendezvous/Buffered: sender e receiver são o mesmo handle.
        _ => handle,
    };

    // 3. Alocar tupla (handle, rx_handle) na arena — 16 bytes.
    let size = ctx.builder.ins().iconst(I64, 16);
    let alloc_fref = get_ffi(ctx, "kata_rt_arena_alloc")?;
    let alloc_inst = ctx.builder.ins().call(alloc_fref, &[rt_val, arena, size]);
    let ptr = ctx.builder.inst_results(alloc_inst)[0];

    // 4. Store dos dois handles na tupla.
    let flags = MemFlagsData::new();
    ctx.builder.ins().store(flags, handle, ptr, 0);
    ctx.builder.ins().store(flags, rx_handle, ptr, 8);

    Ok(ptr)
}

/// Lowera `TypedExprKind::ReceiverFactoryCall` (`rxf!()`).
///
/// O `factory` avalia para o handle de `ReceiverFactory::T` (tag 0b10).
/// Chama `kata_rt_broadcast_receiver_create(arena, factory_handle)` e
/// retorna o handle do novo `Receiver::T` (tag 0b11). Diferente de
/// `lower_channel_create` para `Broadcast`, esta **não** cria um novo
/// `BroadcastInner` — pede um receiver ao factory existente.
pub(crate) fn lower_receiver_factory_call(
    expr: &TypedExpr,
    factory: &kata_ast::Spanned<TypedExpr>,
    _elem_ty: &Ty,
    ctx: &mut LowerCtx,
) -> Result<Value, super::super::CodegenError> {
    // Arena onde o BroadcastReceiver é alocado — fiber_arena (mesmo motivo
    // do channel_create: canais fluem descendente, criador é always-last).
    let arena = ctx
        .fiber_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // Lowera o factory (Ident do rxf) → handle i64 (tag 0b10).
    let factory_handle = super::super::expr::lower_expr(&factory.node, ctx)?;

    // Chama kata_rt_broadcast_receiver_create(arena, factory_handle) → rx_handle.
    let fref = get_ffi(ctx, "kata_rt_broadcast_receiver_create")?;
    let inst = ctx.builder.ins().call(fref, &[arena, factory_handle]);
    let rx_handle = ctx.builder.inst_results(inst)[0];

    // ReceiverFactoryCall retorna o handle do receiver (i64), não uma tupla.
    // O typeck já marcou o tipo como `Receiver::T` — o codegen trata como i64.
    let _ = expr; // silencia unused
    Ok(rx_handle)
}

/// Lowera `TypedExprKind::ChannelSend` (`tx !> valor`).
///
/// Chama `kata_rt_channel_send(handle, value)` e retorna Unit.
pub(crate) fn lower_channel_send(
    channel: &kata_ast::Spanned<TypedExpr>,
    value: &kata_ast::Spanned<TypedExpr>,
    ctx: &mut LowerCtx,
) -> Result<Value, super::super::CodegenError> {
    let handle = super::super::expr::lower_expr(&channel.node, ctx)?;
    let val = super::super::expr::lower_expr(&value.node, ctx)?;

    let fref = get_ffi(ctx, "kata_rt_channel_send")?;
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
) -> Result<Value, super::super::CodegenError> {
    let handle = super::super::expr::lower_expr(&channel.node, ctx)?;

    let fref = get_ffi(ctx, "kata_rt_channel_recv")?;
    let inst = ctx.builder.ins().call(fref, &[handle]);
    let val = ctx.builder.inst_results(inst)[0];

    // Criar binding no var_map (igual ao Let lowering).
    let clif_ty = super::super::resolve_clif_ty(recv_ty, ctx.struct_registry);
    let var = ctx.new_var(bind_name, clif_ty);
    ctx.builder.def_var(var, val);

    Ok(val)
}
