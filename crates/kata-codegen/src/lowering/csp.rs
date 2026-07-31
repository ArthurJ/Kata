//! Lowering de operações CSP — canais, fork, select.
//!
//! Lowera `ChannelCreate`, `ChannelSend`, `ChannelRecv`, `Fork`, e `Select`
//! da TAST para chamadas FFI do runtime.
//!
//! - `channel!()` / `queue!(N)` / `broadcast!()` → FFI de criação + tupla (tx, rx)
//! - `tx !> valor` → `kata_rt_channel_send(handle, value)`
//! - `rx <! nome` → `kata_rt_channel_recv(handle)` + binding no var_map
//! - `fork!(action, args)` → `kata_rt_spawn(fn_ptr, caller_arena, args_ptr)`
//! - `select` → `kata_rt_select(handles, N, timeout_ms)` + dispatch por índice

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, GlobalValueData, InstBuilder, MemFlagsData, Signature};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use kata_core::ty::Ty;
use kata_inference::{ChannelKind, TypedExpr, TypedExprKind, TypedSelectArm};

use super::LowerCtx;

/// Verifica se um tipo é ARC-managed (composto, alocado na arena).
/// Primitivos (Int, Float, Boolean, Unit) são inline e não precisam de ARC.
/// Text e Rational são ponteiros mas vivem na fiber/caller arena (não na root).
/// Para canal, apenas tipos compostos estruturais (Tuple, Struct, List, etc.)
/// são ARC-managed na root_arena.
fn is_arc_type(ty: &Ty) -> bool {
    match ty {
        Ty::Prim(_) | Ty::Unit | Ty::Var(_) | Ty::InferVar(_) | Ty::Action(..) => false,
        // Text/Rational são ponteiros mas não têm header ARC.
        // Sender/Receiver são handles (i64), não ponteiros ARC.
        // Function é fn_ptr, não ponteiro ARC.
        Ty::Sender(_) | Ty::Receiver(_) | Ty::Function(..) => false,
        // Compostos estruturais — alocados na arena, ARC-managed.
        _ => true,
    }
}

/// Extrai o `type_id` do tipo de elemento do canal a partir do tipo do
/// `ChannelCreate` (`Tuple([Sender(T), Receiver(T)])`). O `elem_ty` no
/// `ChannelCreate` pode ser `Var("T0")` não-resolvido; o tipo concreto
/// está em `expr.ty`.
fn lookup_type_id(expr: &TypedExpr, ctx: &LowerCtx) -> i64 {
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
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    // Arena onde o canal é alocado — fiber_arena do criador. Canais fluem
    // apenas descendente (pai → filho via fork!/spawn!) e structured concurrency
    // garante que o criador é always-last. bump.reset() no epílogo libera
    // o canal — O(1), sem leak. Valores ARC-managed enviados pelo canal
    // continuam na root_arena (escape analysis marca Heap no channel_send).
    let arena = ctx
        .fiber_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // 1. Chamar a FFI de criação conforme o kind e backend (in-process vs IPC).
    let handle = match kind {
        ChannelKind::Rendezvous => {
            let ffi_name = if cross_process {
                "kata_rt_ipc_channel_create"
            } else {
                "kata_rt_channel_create"
            };
            let fref = ctx
                .ffi_refs
                .get(ffi_name)
                .copied()
                .ok_or_else(|| super::CodegenError::FfiSymbolNotFound(ffi_name.into()))?;
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
                let fref = ctx
                    .ffi_refs
                    .get("kata_rt_ipc_queue_create")
                    .copied()
                    .ok_or_else(|| {
                        super::CodegenError::FfiSymbolNotFound("kata_rt_ipc_queue_create".into())
                    })?;
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
                let broker_fid = synthesize_ipc_broker(ctx)?;
                let broker_args_ptr = build_broker_args(ctx, queue_rx, ack_rx, ipc_data_tx)?;
                let broker_fn_ptr = {
                    let func_ref = ctx
                        .module
                        .declare_func_in_func(broker_fid, ctx.builder.func);
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
                    ctx.builder
                        .ins()
                        .global_value(ctx.module.target_config().pointer_type(), func_gv)
                };
                let caller_arena_val = ctx
                    .caller_arena
                    .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));
                let spawn_ref = ctx.ffi_refs.get("kata_rt_spawn").copied().ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_spawn".into())
                })?;
                ctx.builder.ins().call(
                    spawn_ref,
                    &[broker_fn_ptr, caller_arena_val, broker_args_ptr],
                );

                // A tupla que o usuário vê é (queue_tx, ipc_data_rx).
                // Alocar tupla de 2 handles na arena (16 bytes).
                let tup_size = ctx.builder.ins().iconst(I64, 16);
                let alloc_fref = ctx
                    .ffi_refs
                    .get("kata_rt_arena_alloc")
                    .copied()
                    .ok_or_else(|| {
                        super::CodegenError::FfiSymbolNotFound("kata_rt_arena_alloc".into())
                    })?;
                let alloc_inst = ctx.builder.ins().call(alloc_fref, &[arena, tup_size]);
                let tup_ptr = ctx.builder.inst_results(alloc_inst)[0];
                ctx.builder.ins().store(flags, queue_tx, tup_ptr, 0);
                ctx.builder.ins().store(flags, ipc_data_rx, tup_ptr, 8);
                return Ok(tup_ptr);
            }
            // In-process buffered (não cross_process).
            let fref = ctx
                .ffi_refs
                .get("kata_rt_queue_create")
                .copied()
                .ok_or_else(|| {
                    super::CodegenError::FfiSymbolNotFound("kata_rt_queue_create".into())
                })?;
            let inst = ctx.builder.ins().call(fref, &[arena, cap_val]);
            ctx.builder.inst_results(inst)[0]
        }
        ChannelKind::Broadcast => {
            if cross_process {
                return Err(super::CodegenError::UnsupportedNode(
                    "broadcast!() cross-process não suportado — o child não vê writes do parent após fork (COW copy)".into(),
                ));
            }
            // Broadcast in-process.
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
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    // Arena onde o BroadcastReceiver é alocado — fiber_arena (mesmo motivo
    // do channel_create: canais fluem descendente, criador é always-last).
    let arena = ctx
        .fiber_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // Lowera o factory (Ident do rxf) → handle i64 (tag 0b10).
    let factory_handle = super::expr::lower_expr(&factory.node, ctx)?;

    // Chama kata_rt_broadcast_receiver_create(arena, factory_handle) → rx_handle.
    let fref = ctx
        .ffi_refs
        .get("kata_rt_broadcast_receiver_create")
        .copied()
        .ok_or_else(|| {
            super::CodegenError::FfiSymbolNotFound("kata_rt_broadcast_receiver_create".into())
        })?;
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
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let handle = super::expr::lower_expr(&channel.node, ctx)?;
    let val = super::expr::lower_expr(&value.node, ctx)?;

    // incref + decref no channel send formam um par:
    // - alloc cria com refcount=1 (referência do sender)
    // - incref → refcount=2 (referência do receiver via canal)
    // - send entrega o ponteiro ao canal
    // - decref → refcount=1 (sender libera sua referência temporária)
    // O receiver recebe com refcount=1. Quando consome e decrementa,
    // refcount → 0 e o bloco inteiro (header + dados) é desalocado.
    crate::lowering::escape_arena::incref_if_heap(value.node.escape, val, ctx)?;

    let fref = ctx
        .ffi_refs
        .get("kata_rt_channel_send")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_channel_send".into()))?;
    ctx.builder.ins().call(fref, &[handle, val]);

    // decref da referência temporária do sender.
    crate::lowering::escape_arena::decref_if_heap(value.node.escape, val, ctx)?;

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
    let clif_ty = super::resolve_clif_ty(recv_ty, ctx.struct_registry);
    let var = ctx.new_var(bind_name, clif_ty);
    ctx.builder.def_var(var, val);
    // Valor recebido via canal: se o tipo é composto, é ARC-managed (Heap).
    // Registrar para decref no epílogo da action.
    if is_arc_type(recv_ty) {
        ctx.arc_vars.push(var);
    }

    Ok(val)
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
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    // 1. Lowerar args (tupla) → args_ptr.
    let args_ptr = super::expr::lower_expr(&args.node, ctx)?;

    // 2. Obter fn_ptr:
    //    - Fork direto: lookup em kata_ids por action_name, GlobalValue::Symbol.
    //    - Fork indireto: lower action_expr → runtime fn_ptr value.
    let fn_ptr = if action_name == "__indirect_fork" {
        // Indirect fork — lower action_expr to get fn_ptr at runtime.
        super::expr::lower_expr(&action_expr.node, ctx)?
    } else {
        // Direct fork — lookup by action_name in kata_ids.
        // Extrair param_types do args (tupla tipada) para lookup no kata_ids.
        let param_types: Vec<Ty> = match &args.node.kind {
            TypedExprKind::Unit => Vec::new(),
            TypedExprKind::Tuple { elements } => {
                elements.iter().map(|e| e.node.ty.clone()).collect()
            }
            _ => vec![args.node.ty.clone()],
        };

        // Procurar a Action em kata_ids por (name, param_types).
        let mut found_key: Option<super::module::FuncKey> = None;
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
            super::CodegenError::UnsupportedNode(format!(
                "fork!: Action `{action_name}` não encontrada em kata_ids"
            ))
        })?;

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
        ctx.builder
            .ins()
            .global_value(ctx.module.target_config().pointer_type(), func_gv)
    };

    // 5. Determinar caller_arena (onde os args vivem — EscapeTarget do expr).
    let caller_arena_val = crate::lowering::escape_arena::arena_handle_for_escape(expr.escape, ctx);

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
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    // 1. Lowerar args (tupla) → args_ptr.
    let args_ptr = super::expr::lower_expr(&args.node, ctx)?;

    // 2. Obter fn_ptr (mesma lógica de lower_fork).
    let fn_ptr = if action_name.starts_with("__indirect") {
        super::expr::lower_expr(&action_expr.node, ctx)?
    } else {
        let param_types: Vec<Ty> = match &args.node.kind {
            TypedExprKind::Unit => Vec::new(),
            TypedExprKind::Tuple { elements } => {
                elements.iter().map(|e| e.node.ty.clone()).collect()
            }
            _ => vec![args.node.ty.clone()],
        };

        let mut found_key: Option<super::module::FuncKey> = None;
        for key in ctx.kata_ids.keys() {
            if key.0 == action_name && key.1 == param_types {
                found_key = Some(key.clone());
                break;
            }
        }
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
                "spawn!: Action `{action_name}` não encontrada em kata_ids"
            ))
        })?;

        let callee_fid = ctx.kata_ids.get(&key).copied().ok_or_else(|| {
            super::CodegenError::UnsupportedNode(format!(
                "spawn!: FuncId para Action `{action_name}` não encontrado"
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
        ctx.builder
            .ins()
            .global_value(ctx.module.target_config().pointer_type(), func_gv)
    };

    // 3. Determinar arena — usar caller_arena (root_arena no entry point).
    //    O fork() faz COW da arena do caller, e o child executa com essa arena.
    let arena_val = ctx
        .caller_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // 4. kata_rt_spawn_process(fn_ptr, args_ptr, arena) — fork e exec.
    //    Fire-and-forget: não há pipe de resultado, não há return.
    let spawn_ref = ctx
        .ffi_refs
        .get("kata_rt_spawn_process")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_spawn_process".into()))?;
    ctx.builder
        .ins()
        .call(spawn_ref, &[fn_ptr, args_ptr, arena_val]);

    // 5. Retorna Unit (fire-and-forget como fork!).
    Ok(ctx.builder.ins().iconst(I64, 0))
}

/// Lowera `TypedExprKind::Select` — multiplexação de recebimento de canais
/// com timeout opcional.
///
/// Estrutura do lowering:
///
/// 1. Lowerar cada `arm.channel` → array de handles na arena (N * 8 bytes)
/// 2. Lowerar `timeout_ms` se presente (iconst -1 se ausente)
/// 3. Chamar `kata_rt_select(handles_ptr, N, timeout_ms)` → idx (i64)
/// 4. Branch chain: `if idx == 0 { arm_0 } else if idx == 1 { arm_1 } ...`
///    - Cada braço: `kata_rt_channel_recv(handles[i])` → binding → body
///    - Se `idx == -2` (SELECT_TIMEOUT): lowerar `timeout_body` (se presente)
/// 5. cont_block junta o resultado de todos os braços
///
/// O tipo de retorno é o tipo unificado de todos os braços e do timeout_body.
pub(crate) fn lower_select(
    expr: &TypedExpr,
    arms: &[TypedSelectArm],
    timeout_ms: &Option<Box<kata_ast::Spanned<TypedExpr>>>,
    timeout_body: &Option<Box<kata_ast::Spanned<TypedExpr>>>,
    ctx: &mut LowerCtx,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let n_arms = arms.len() as i64;
    let ret_clif = super::resolve_clif_ty(&expr.ty, ctx.struct_registry);

    // Arena para alocar o array de handles — fiber_arena (consistente com
    // channel_create: o array é efêmero, vive apenas durante o select).
    let arena = ctx
        .fiber_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // 1. Alocar array de N handles na arena (N * 8 bytes).
    let array_size = ctx.builder.ins().iconst(I64, n_arms * 8);
    let alloc_fref = ctx
        .ffi_refs
        .get("kata_rt_arena_alloc")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_arena_alloc".into()))?;
    let alloc_inst = ctx.builder.ins().call(alloc_fref, &[arena, array_size]);
    let handles_ptr = ctx.builder.inst_results(alloc_inst)[0];

    // 2. Lowerar cada arm.channel e store no array.
    let flags = MemFlagsData::new();
    let mut handle_values: Vec<cranelift_codegen::ir::Value> = Vec::with_capacity(arms.len());
    for (i, arm) in arms.iter().enumerate() {
        let handle = super::expr::lower_expr(&arm.channel.node, ctx)?;
        handle_values.push(handle);
        let offset = (i as i32) * 8;
        ctx.builder.ins().store(flags, handle, handles_ptr, offset);
    }

    // 3. Lowerar timeout_ms (se presente) ou -1 (sem timeout).
    //    O timeout_ms é uma TypedExpr Int — lower_expr retorna SMI-tagged.
    //    A FFI espera o valor cru em ms, então fazemos untag: val >> 1.
    let timeout_val = if let Some(tm) = timeout_ms {
        let smi_val = super::expr::lower_expr(&tm.node, ctx)?;
        // Untag SMI: (val << 1 | 1) >> 1 = val. Para SMIs positivos (timeout
        // sempre positivo), ushr_imm é seguro.
        ctx.builder.ins().ushr_imm(smi_val, 1)
    } else {
        ctx.builder.ins().iconst(I64, -1)
    };

    // 4. Chamar kata_rt_select(handles_ptr, N, timeout_ms) → idx.
    let select_fref = ctx
        .ffi_refs
        .get("kata_rt_select")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_select".into()))?;
    let n_val = ctx.builder.ins().iconst(I64, n_arms);
    let select_inst = ctx
        .builder
        .ins()
        .call(select_fref, &[handles_ptr, n_val, timeout_val]);
    let idx = ctx.builder.inst_results(select_inst)[0];

    // 5. Branch chain: dispatch por índice.
    //    cont_block recebe o resultado de qualquer braço.
    let cont_block = ctx.builder.create_block();
    ctx.builder.append_block_param(cont_block, ret_clif);

    let mut next_test_block = ctx.builder.create_block();
    ctx.builder.ins().jump(next_test_block, &[]);
    ctx.builder.seal_block(next_test_block);

    // Braços de canal: idx == 0, idx == 1, ...
    for (i, arm) in arms.iter().enumerate() {
        ctx.builder.switch_to_block(next_test_block);

        let body_block = ctx.builder.create_block();
        next_test_block = ctx.builder.create_block();

        // Comparar idx == i.
        let expected = ctx.builder.ins().iconst(I64, i as i64);
        let is_this = ctx.builder.ins().icmp(
            cranelift_codegen::ir::condcodes::IntCC::Equal,
            idx,
            expected,
        );
        ctx.builder
            .ins()
            .brif(is_this, body_block, &[], next_test_block, &[]);
        ctx.builder.seal_block(next_test_block);
        ctx.builder.seal_block(body_block);

        // No body_block: recv do canal i, criar binding, lowerar body.
        ctx.builder.switch_to_block(body_block);
        ctx.emitted_tail_call = false;

        let recv_fref = ctx
            .ffi_refs
            .get("kata_rt_channel_recv")
            .copied()
            .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_channel_recv".into()))?;
        let recv_inst = ctx.builder.ins().call(recv_fref, &[handle_values[i]]);
        let recv_val = ctx.builder.inst_results(recv_inst)[0];

        // Criar binding do braço (igual lower_channel_recv).
        let clif_ty = super::resolve_clif_ty(&arm.recv_ty, ctx.struct_registry);
        let var = ctx.new_var(&arm.bind_name, clif_ty);
        ctx.builder.def_var(var, recv_val);

        // Lowerar body do braço.
        let body_val = super::expr::lower_expr(&arm.body.node, ctx)?;

        let is_terminator = matches!(
            arm.body.node.kind,
            TypedExprKind::Break | TypedExprKind::Continue | TypedExprKind::Return(_)
        );
        if !is_terminator && !ctx.emitted_tail_call {
            ctx.builder.ins().jump(
                cont_block,
                &[cranelift_codegen::ir::BlockArg::Value(body_val)],
            );
        }
    }

    // Último braço: timeout (idx == -2) se timeout_body presente,
    // ou trap se não tem timeout (não deveria acontecer em código válido).
    ctx.builder.switch_to_block(next_test_block);

    if let Some(tb) = timeout_body {
        let timeout_block = ctx.builder.create_block();
        let trap_block = ctx.builder.create_block();

        // idx == -2 (SELECT_TIMEOUT).
        let timeout_sentinel = ctx.builder.ins().iconst(I64, -2);
        let is_timeout = ctx.builder.ins().icmp(
            cranelift_codegen::ir::condcodes::IntCC::Equal,
            idx,
            timeout_sentinel,
        );
        ctx.builder
            .ins()
            .brif(is_timeout, timeout_block, &[], trap_block, &[]);
        ctx.builder.seal_block(timeout_block);
        ctx.builder.seal_block(trap_block);

        // Lowerar timeout_body.
        ctx.builder.switch_to_block(timeout_block);
        ctx.emitted_tail_call = false;
        let tb_val = super::expr::lower_expr(&tb.node, ctx)?;

        let is_terminator = matches!(
            tb.node.kind,
            TypedExprKind::Break | TypedExprKind::Continue | TypedExprKind::Return(_)
        );
        if !is_terminator && !ctx.emitted_tail_call {
            ctx.builder.ins().jump(
                cont_block,
                &[cranelift_codegen::ir::BlockArg::Value(tb_val)],
            );
        }

        // trap_block: idx não reconhecido (impossível em código válido).
        ctx.builder.switch_to_block(trap_block);
        ctx.builder
            .ins()
            .trap(cranelift_codegen::ir::TrapCode::user(1).expect("trap code 1 é sempre válido"));
    } else {
        // Sem timeout_body: idx == -2 não deveria acontecer (select sem
        // timeout não retorna -2). Trap por segurança.
        ctx.builder
            .ins()
            .trap(cranelift_codegen::ir::TrapCode::user(1).expect("trap code 1 é sempre válido"));
    }

    // cont_block: resultado do select.
    ctx.builder.seal_block(cont_block);
    ctx.builder.switch_to_block(cont_block);
    let result = ctx.builder.block_params(cont_block)[0];

    ctx.emitted_tail_call = false;
    Ok(result)
}

// ─────────────────────────────────────────────────────────────────────
// IPC Broker — síntese de Action JIT para Buffered cross-process
// ─────────────────────────────────────────────────────────────────────

/// Nome único do broker no JITModule.
const BROKER_NAME: &str = "__kata_ipc_broker";

/// Timeout do select do broker (30 segundos). Se o child morrer ou a queue
/// ficar vazia por 30s, o broker termina (return 0).
const BROKER_TIMEOUT_MS: i64 = 30_000;

/// Sintetiza a função JIT do broker IPC como uma Action separada.
///
/// O broker é uma função JIT com a assinatura uniforme de Action:
/// `(fiber_arena: i64, caller_arena: i64, args_ptr: i64) -> i64`
/// com `CallConv::Tail` (igual às Actions definidas pelo usuário).
///
/// `args_ptr` aponta para 3 handles i64 na arena:
///   `[queue_rx_handle, ack_rx_handle, ipc_data_tx_handle]`
///
/// Corpo do broker:
/// 1. Load dos 3 handles de args_ptr
/// 2. Alocar array de 2 handles na fiber_arena: `[queue_rx, ack_rx]`
/// 3. Loop:
///    a. `kata_rt_select(handles_ptr, 2, BROKER_TIMEOUT_MS)` → idx
///    b. idx == 0 (queue_rx pronto): `channel_recv(queue_rx)` → val;
///       `channel_send(ipc_data_tx, val)`; continua loop
///    c. idx == 1 (ack_rx pronto): `channel_recv(ack_rx)` → descarta;
///       continua loop
///    d. idx == -2 (SELECT_TIMEOUT): `return 0` (broker termina)
///    e. idx == -1 (WOULD_BLOCK): continua loop
/// 4. `return 0`
///
/// Declara e define a função no `ctx.module` (JITModule), retornando o
/// `FuncId` para que o caller obtenha o fn_ptr via `GlobalValue::Symbol`.
pub(crate) fn synthesize_ipc_broker(
    ctx: &mut LowerCtx,
) -> Result<cranelift_module::FuncId, super::CodegenError> {
    // Idempotência: se o broker já foi sintetizado neste módulo, reutiliza.
    if let Some(fid) = ctx.ipc_broker_fid {
        return Ok(fid);
    }

    // Declara a função no module (sem definir o corpo ainda).
    // Assinatura uniforme de Action: (fiber_arena, caller_arena, args_ptr) -> i64.
    let mut sig = Signature::new(CallConv::Tail);
    sig.params.push(AbiParam::new(I64)); // fiber_arena
    sig.params.push(AbiParam::new(I64)); // caller_arena
    sig.params.push(AbiParam::new(I64)); // args_ptr
    sig.returns.push(AbiParam::new(I64)); // sempre i64

    let func_id = ctx
        .module
        .declare_function(BROKER_NAME, Linkage::Local, &sig)
        .map_err(|e| super::CodegenError::Cranelift(format!("declare {BROKER_NAME}: {e}")))?;

    // Cria Context + FunctionBuilder para construir o corpo do broker.
    let mut fn_ctx = ctx.module.make_context();
    let metadata = crate::metadata::MetadataTable::new();

    {
        let func_ir = &mut fn_ctx.func;
        func_ir.signature = sig;

        // Declara FFI no Function do broker (precisa dos FuncRefs locais).
        let mut ffi_refs: HashMap<String, cranelift_codegen::ir::FuncRef> = HashMap::new();
        for (fname, &fid) in ctx.ffi_ids {
            let fref = ctx.module.declare_func_in_func(fid, func_ir);
            ffi_refs.insert(fname.clone(), fref);
        }

        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(func_ir, &mut builder_ctx);

        // ── Entry block ──
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let params: Vec<cranelift_codegen::ir::Value> = builder.block_params(entry).to_vec();
        let fiber_arena = params[0];
        let _caller_arena = params[1]; // não usado pelo broker
        let args_ptr = params[2];

        // Helpers: FuncRefs para as FFIs que o broker usa.
        let flags = MemFlagsData::new();
        let select_ref = ffi_refs
            .get("kata_rt_select")
            .copied()
            .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_select".into()))?;
        let recv_ref = ffi_refs
            .get("kata_rt_channel_recv")
            .copied()
            .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_channel_recv".into()))?;
        let send_ref = ffi_refs
            .get("kata_rt_channel_send")
            .copied()
            .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_channel_send".into()))?;
        let alloc_ref = ffi_refs
            .get("kata_rt_arena_alloc")
            .copied()
            .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_arena_alloc".into()))?;

        // 1. Load dos 3 handles de args_ptr.
        let queue_rx = builder.ins().load(I64, flags, args_ptr, 0);
        let ack_rx = builder.ins().load(I64, flags, args_ptr, 8);
        let ipc_data_tx = builder.ins().load(I64, flags, args_ptr, 16);

        // 2. Alocar array de 2 handles na fiber_arena: [queue_rx, ack_rx].
        let size_16 = builder.ins().iconst(I64, 16);
        let alloc_inst = builder.ins().call(alloc_ref, &[fiber_arena, size_16]);
        let handles_ptr = builder.inst_results(alloc_inst)[0];
        builder.ins().store(flags, queue_rx, handles_ptr, 0);
        builder.ins().store(flags, ack_rx, handles_ptr, 8);

        // Constantes do loop.
        let n_handles = builder.ins().iconst(I64, 2);
        let timeout_val = builder.ins().iconst(I64, BROKER_TIMEOUT_MS);
        let zero = builder.ins().iconst(I64, 0);
        let one = builder.ins().iconst(I64, 1);
        let neg_two = builder.ins().iconst(I64, -2);

        // 3. Loop.
        let loop_header = builder.create_block();
        let exit_block = builder.create_block();
        // NÃO selar loop_header nem exit_block ainda — têm back-edges
        // que só serão adicionados no corpo do loop.

        builder.ins().jump(loop_header, &[]);
        builder.switch_to_block(loop_header);

        // a. select(handles_ptr, 2, timeout) → idx
        let sel_inst = builder
            .ins()
            .call(select_ref, &[handles_ptr, n_handles, timeout_val]);
        let idx = builder.inst_results(sel_inst)[0];

        // b. idx == 0 → queue_rx pronto
        let is_queue = builder.ins().icmp(IntCC::Equal, idx, zero);
        let queue_block = builder.create_block();
        let after_queue = builder.create_block();
        builder
            .ins()
            .brif(is_queue, queue_block, &[], after_queue, &[]);
        builder.seal_block(queue_block);
        builder.seal_block(after_queue);

        // queue_block: recv(queue_rx) → val; send(ipc_data_tx, val); → loop_header
        builder.switch_to_block(queue_block);
        let recv_inst = builder.ins().call(recv_ref, &[queue_rx]);
        let val = builder.inst_results(recv_inst)[0];
        builder.ins().call(send_ref, &[ipc_data_tx, val]);
        builder.ins().jump(loop_header, &[]);

        // after_queue: idx == 1 → ack_rx pronto (ou timeout, ou would_block)
        builder.switch_to_block(after_queue);
        let is_ack = builder.ins().icmp(IntCC::Equal, idx, one);
        let ack_block = builder.create_block();
        let after_ack = builder.create_block();
        builder.ins().brif(is_ack, ack_block, &[], after_ack, &[]);
        builder.seal_block(ack_block);
        builder.seal_block(after_ack);

        // ack_block: recv(ack_rx) → descarta; → loop_header
        builder.switch_to_block(ack_block);
        builder.ins().call(recv_ref, &[ack_rx]);
        builder.ins().jump(loop_header, &[]);

        // after_ack: idx == -2 → timeout → exit; else (-1 would_block) → loop
        builder.switch_to_block(after_ack);
        let is_timeout = builder.ins().icmp(IntCC::Equal, idx, neg_two);
        builder
            .ins()
            .brif(is_timeout, exit_block, &[], loop_header, &[]);

        // Agora todos os predecessores de loop_header e exit_block são conhecidos.
        builder.seal_block(loop_header);
        builder.seal_block(exit_block);

        // exit_block: return 0
        builder.switch_to_block(exit_block);
        let ret_zero = builder.ins().iconst(I64, 0);
        builder.ins().return_(&[ret_zero]);

        builder.finalize();
    }

    // Define a função no module.
    ctx.module
        .define_function(func_id, &mut fn_ctx)
        .map_err(|e| super::CodegenError::Cranelift(format!("define {BROKER_NAME}: {e}")))?;
    ctx.module.clear_context(&mut fn_ctx);

    let _ = metadata; // silencia unused — reservado para futura instrumentação
    ctx.ipc_broker_fid = Some(func_id);
    Ok(func_id)
}

/// Constrói o args_ptr do broker: 3 handles i64 na arena.
///
/// `args_ptr` aponta para `[queue_rx, ack_rx, ipc_data_tx]` (24 bytes).
/// Alocado na `fiber_arena` do caller (a arena onde os handles vivem).
pub(crate) fn build_broker_args(
    ctx: &mut LowerCtx,
    queue_rx: cranelift_codegen::ir::Value,
    ack_rx: cranelift_codegen::ir::Value,
    ipc_data_tx: cranelift_codegen::ir::Value,
) -> Result<cranelift_codegen::ir::Value, super::CodegenError> {
    let arena = ctx
        .fiber_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // Alocar 24 bytes (3 * 8) na fiber_arena.
    let size = ctx.builder.ins().iconst(I64, 24);
    let alloc_fref = ctx
        .ffi_refs
        .get("kata_rt_arena_alloc")
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound("kata_rt_arena_alloc".into()))?;
    let alloc_inst = ctx.builder.ins().call(alloc_fref, &[arena, size]);
    let ptr = ctx.builder.inst_results(alloc_inst)[0];

    // Store dos 3 handles.
    let flags = MemFlagsData::new();
    ctx.builder.ins().store(flags, queue_rx, ptr, 0);
    ctx.builder.ins().store(flags, ack_rx, ptr, 8);
    ctx.builder.ins().store(flags, ipc_data_tx, ptr, 16);

    Ok(ptr)
}
