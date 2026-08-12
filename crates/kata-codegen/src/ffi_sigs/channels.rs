//! Assinaturas FFI para canais CSP, queues, broadcast, IPC, select e logging.

use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{AbiParam, Signature};
use crate::call_conv::ffi_call_conv;
use kata_core::ffi::FfiSymbol;

/// Constrói a assinatura para símbolos de canais/IPC/select/logging.
/// Retorna `Some(sig)` se `sym` pertence a esta categoria, `None` caso contrário.
pub(crate) fn sig_for(sym: FfiSymbol) -> Option<Signature> {
    let mut sig = Signature::new(ffi_call_conv());
    match sym {
        // ── Canais CSP ──
        // channel_create: (arena) -> handle
        FfiSymbol::ChannelCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        // queue_create: (arena, capacity, policy) -> handle
        FfiSymbol::QueueCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.params.push(AbiParam::new(I64)); // capacity
            sig.params.push(AbiParam::new(I64)); // policy (0=Block, 1=Drop)
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        // broadcast_create: (arena) -> handle
        FfiSymbol::BroadcastCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        // broadcast_receiver_create: (arena, factory_handle) -> handle
        FfiSymbol::BroadcastReceiverCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.params.push(AbiParam::new(I64)); // factory_handle
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        // ipc_channel_create: (arena, type_id, ack_tx_handle) -> handle
        FfiSymbol::IpcChannelCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.params.push(AbiParam::new(I64)); // type_id
            sig.params.push(AbiParam::new(I64)); // ack_tx_handle
            sig.returns.push(AbiParam::new(I64)); // handle
        }
        // ipc_queue_create: (arena, cap, type_id) -> ptr (6 handles na arena)
        FfiSymbol::IpcQueueCreate => {
            sig.params.push(AbiParam::new(I64)); // arena
            sig.params.push(AbiParam::new(I64)); // cap
            sig.params.push(AbiParam::new(I64)); // type_id
            sig.returns.push(AbiParam::new(I64)); // ptr to 6-handle tuple
        }
        // channel_send: (handle, value) -> i64 (0=OK, -1=block)
        FfiSymbol::ChannelSend => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.params.push(AbiParam::new(I64)); // value
            sig.returns.push(AbiParam::new(I64)); // status
        }
        // channel_recv: (handle) -> i64 (valor ou -1=block)
        FfiSymbol::ChannelRecv => {
            sig.params.push(AbiParam::new(I64)); // handle
            sig.returns.push(AbiParam::new(I64)); // value
        }
        // select: (handles_ptr, n_handles, timeout_ms) -> i64 (idx, -1=block, -2=timeout)
        FfiSymbol::ChannelSelect => {
            sig.params.push(AbiParam::new(I64)); // handles ptr
            sig.params.push(AbiParam::new(I64)); // n_handles
            sig.params.push(AbiParam::new(I64)); // timeout_ms (<=0 = sem timeout)
            sig.returns.push(AbiParam::new(I64)); // index or sentinel
        }
        // select_files: (handles_ptr, n_handles) -> i64 (idx, -1=block)
        FfiSymbol::SelectFiles => {
            sig.params.push(AbiParam::new(I64)); // handles ptr
            sig.params.push(AbiParam::new(I64)); // n_handles
            sig.returns.push(AbiParam::new(I64)); // index or sentinel
        }
        // select_combined: (chan_ptr, n_c, file_ptr, n_f, socket_ptr, n_s, timeout_ms) -> i64
        // Retorna índice global: 0..n_c-1 = channel, n_c..n_c+n_f-1 = file,
        // n_c+n_f..n_c+n_f+n_s-1 = socket.
        // -1 = WOULD_BLOCK, -2 = SELECT_TIMEOUT.
        FfiSymbol::SelectCombined => {
            sig.params.push(AbiParam::new(I64)); // chan_handles ptr
            sig.params.push(AbiParam::new(I64)); // n_c
            sig.params.push(AbiParam::new(I64)); // file_handles ptr
            sig.params.push(AbiParam::new(I64)); // n_f
            sig.params.push(AbiParam::new(I64)); // socket_handles ptr
            sig.params.push(AbiParam::new(I64)); // n_s
            sig.params.push(AbiParam::new(I64)); // timeout_ms (<=0 = sem timeout)
            sig.returns.push(AbiParam::new(I64)); // global index or sentinel
        }
        // log_publish: (topic_ptr, level, msg, policy_ptr) -> i64 (0=OK, -1=erro)
        FfiSymbol::LogPublish => {
            sig.params.push(AbiParam::new(I64)); // topic_ptr (handle Text ou 0)
            sig.params.push(AbiParam::new(I64)); // level (tag do enum LogLevel)
            sig.params.push(AbiParam::new(I64)); // msg (handle Text)
            sig.params.push(AbiParam::new(I64)); // policy_ptr (handle Text ou 0)
            sig.returns.push(AbiParam::new(I64)); // status
        }
        // log_recv: (topic_ptr) -> i64 (valor ou 0 se canal fechou)
        FfiSymbol::LogRecv => {
            sig.params.push(AbiParam::new(I64)); // topic_ptr (handle Text ou 0)
            sig.returns.push(AbiParam::new(I64)); // value
        }
        // log_config: (topic_ptr, policy_ptr, level) -> ()
        FfiSymbol::LogConfig => {
            sig.params.push(AbiParam::new(I64)); // topic_ptr (handle Text ou 0)
            sig.params.push(AbiParam::new(I64)); // policy_ptr (handle Text ou 0)
            sig.params.push(AbiParam::new(I64)); // level (tag do enum LogLevel)
            // sem returns — Unit
        }
        _ => return None,
    }
    Some(sig)
}
