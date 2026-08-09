//! IPC Broker — síntese de Action JIT para Buffered cross-process.

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Signature, Value};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};

use super::super::LowerCtx;
use super::get_ffi;

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
///    `channel_send(ipc_data_tx, val)`; continua loop
///    c. idx == 1 (ack_rx pronto): `channel_recv(ack_rx)` → descarta;
///    continua loop
///    d. idx == -2 (SELECT_TIMEOUT): `return 0` (broker termina)
///    e. idx == -1 (WOULD_BLOCK): continua loop
/// 4. `return 0`
///
/// Declara e define a função no `ctx.module` (JITModule), retornando o
/// `FuncId` para que o caller obtenha o fn_ptr via `GlobalValue::Symbol`.
pub(crate) fn synthesize_ipc_broker(
    ctx: &mut LowerCtx,
) -> Result<cranelift_module::FuncId, super::super::CodegenError> {
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
        .map_err(|e| {
            super::super::CodegenError::Cranelift { reason: format!("declare {BROKER_NAME}: {e}") }
        })?;

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

        let params: Vec<Value> = builder.block_params(entry).to_vec();
        let fiber_arena = params[0];
        let _caller_arena = params[1]; // não usado pelo broker
        let args_ptr = params[2];

        // Helpers: FuncRefs para as FFIs que o broker usa.
        let flags = MemFlagsData::new();
        let select_ref = get_ffi_from(&ffi_refs, "kata_rt_select")?;
        let recv_ref = get_ffi_from(&ffi_refs, "kata_rt_channel_recv")?;
        let send_ref = get_ffi_from(&ffi_refs, "kata_rt_channel_send")?;
        let alloc_ref = get_ffi_from(&ffi_refs, "kata_rt_arena_alloc")?;

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
        .map_err(|e| super::super::CodegenError::Cranelift { reason: format!("define {BROKER_NAME}: {e}") })?;
    ctx.module.clear_context(&mut fn_ctx);

    let _ = metadata; // silencia unused — reservado para futura instrumentação
    ctx.ipc_broker_fid = Some(func_id);
    Ok(func_id)
}

/// Busca um `FuncRef` num `HashMap` local de FFI (usado pelo broker, que
/// tem seus próprios FuncRefs declarados no Function do broker, não no
/// `ctx.ffi_refs` do caller).
fn get_ffi_from(
    ffi_refs: &HashMap<String, cranelift_codegen::ir::FuncRef>,
    name: &str,
) -> Result<cranelift_codegen::ir::FuncRef, super::super::CodegenError> {
    ffi_refs
        .get(name)
        .copied()
        .ok_or_else(|| super::super::CodegenError::FfiSymbolNotFound { symbol: name.into() })
}

/// Constrói o args_ptr do broker: 3 handles i64 na arena.
///
/// `args_ptr` aponta para `[queue_rx, ack_rx, ipc_data_tx]` (24 bytes).
/// Alocado na `fiber_arena` do caller (a arena onde os handles vivem).
pub(crate) fn build_broker_args(
    ctx: &mut LowerCtx,
    queue_rx: Value,
    ack_rx: Value,
    ipc_data_tx: Value,
) -> Result<Value, super::super::CodegenError> {
    let arena = ctx
        .fiber_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    // Alocar 24 bytes (3 * 8) na fiber_arena.
    let size = ctx.builder.ins().iconst(I64, 24);
    let alloc_fref = get_ffi(ctx, "kata_rt_arena_alloc")?;
    let alloc_inst = ctx.builder.ins().call(alloc_fref, &[arena, size]);
    let ptr = ctx.builder.inst_results(alloc_inst)[0];

    // Store dos 3 handles.
    let flags = MemFlagsData::new();
    ctx.builder.ins().store(flags, queue_rx, ptr, 0);
    ctx.builder.ins().store(flags, ack_rx, ptr, 8);
    ctx.builder.ins().store(flags, ipc_data_tx, ptr, 16);

    Ok(ptr)
}
