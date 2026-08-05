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

mod broker;
mod channel;
mod fork_spawn;
mod select;

pub(crate) use channel::{
    lower_channel_create, lower_channel_recv, lower_channel_send, lower_receiver_factory_call,
};
pub(crate) use fork_spawn::{lower_fork, lower_spawn};
pub(crate) use select::lower_select;

use cranelift_codegen::ir::FuncRef;

use super::LowerCtx;

/// Busca um `FuncRef` pelo nome da FFI no `ctx.ffi_refs`, retornando
/// `CodegenError::FfiSymbolNotFound` se ausente.
pub(crate) fn get_ffi(
    ctx: &LowerCtx,
    name: &str,
) -> Result<FuncRef, super::CodegenError> {
    ctx.ffi_refs
        .get(name)
        .copied()
        .ok_or_else(|| super::CodegenError::FfiSymbolNotFound(name.into()))
}